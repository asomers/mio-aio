extern crate libc;
extern crate env_logger;
extern crate mio;
extern crate mio_aio;
extern crate nix;
extern crate tempfile;

use mio::{Events, Poll, PollOpt, Token};
use mio::unix::UnixReady;
use tempfile::tempfile;
use std::borrow::Borrow;
use std::os::unix::io::AsRawFd;
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Deref;
use std::rc::Rc;


const UDATA: Token = Token(0xdeadbeef);

#[test]
pub fn test_aio_cancel() {
    const WBUF: &'static [u8] = b"abcdef";
    let f = tempfile().unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    let aiocb = mio_aio::AioCb::from_slice(f.as_raw_fd(),
        0,   //offset
        &WBUF,
        0,   //priority
        mio_aio::LioOpcode::LIO_NOP);
    poll.register(&aiocb, UDATA, UnixReady::aio().into(), PollOpt::empty())
        .expect("registration failed");

    aiocb.write().unwrap();
    aiocb.cancel().ok().expect("aio_cancel failed");

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);
    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_aio());

    // Since we cancelled the I/O, we musn't care whether it succeeded.
    let _ = aiocb.aio_return();
}


#[test]
pub fn test_aio_fsync() {
    const INITIAL: &'static [u8] = b"abcdef123456";
    let mut f = tempfile().unwrap();
    f.write(INITIAL).unwrap();
    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let aiocb = mio_aio::AioCb::from_fd( f.as_raw_fd(), 0);
    poll.register(&aiocb, UDATA, UnixReady::aio().into(), PollOpt::empty())
        .expect("registration failed");

    aiocb.fsync(mio_aio::AioFsyncMode::O_SYNC).unwrap();
    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);
    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_aio());

    aiocb.aio_return().unwrap();
}

#[test]
pub fn test_aio_read() {
    const INITIAL: &'static [u8] = b"abcdef123456";
    let rbuf = Rc::new(vec![0; 4].into_boxed_slice());
    const EXPECT: &'static [u8] = b"cdef";
    let mut f = tempfile().unwrap();
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    {
        let aiocb = mio_aio::AioCb::from_boxed_slice(f.as_raw_fd(),
            2,   //offset
            rbuf.clone(),
            0,   //priority
            mio_aio::LioOpcode::LIO_NOP);
        poll.register(&aiocb, UDATA, UnixReady::aio().into(), PollOpt::empty())
            .ok().expect("registration failed");

        aiocb.read().unwrap();

        poll.poll(&mut events, None).expect("poll failed");
        assert_eq!(events.len(), 1);
        let ev = events.get(0).unwrap();
        assert_eq!(ev.token(), UDATA);
        assert!(UnixReady::from(ev.readiness()).is_aio());

        assert_eq!(aiocb.aio_return().unwrap(), EXPECT.len() as isize);
    }
    assert!(rbuf.deref().deref() == EXPECT);
}

#[test]
pub fn test_aio_write() {
    const WBUF: &'static [u8] = b"abcdef";
    let mut f = tempfile().unwrap();
    let mut rbuf = Vec::new();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    let aiocb = mio_aio::AioCb::from_slice(f.as_raw_fd(),
        0,   //offset
        &WBUF,
        0,   //priority
        mio_aio::LioOpcode::LIO_NOP);
    poll.register(&aiocb, UDATA, UnixReady::aio().into(), PollOpt::empty())
        .expect("registration failed");

    aiocb.write().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);
    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_aio());

    assert_eq!(aiocb.aio_return().unwrap(), WBUF.len() as isize);
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read_to_end(&mut rbuf).unwrap();
    assert!(len == WBUF.len());
    assert!(rbuf == WBUF);
}

#[test]
pub fn test_lio_oneread() {
    const INITIAL: &'static [u8] = b"abcdef123456";
    const EXPECT: &'static [u8] = b"cdef";
    let mut f = tempfile().unwrap();
    let buf = Rc::new(vec![0; 4].into_boxed_slice());
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(1);
    liocb.emplace_boxed_slice(f.as_raw_fd(), 2, buf.clone(), 0,
                              mio_aio::LioOpcode::LIO_READ);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);
    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb[0].error().unwrap();
    assert_eq!(liocb[0].aio_return().unwrap(), EXPECT.len() as isize);
    assert!(buf.deref().deref() == EXPECT);
}

#[test]
pub fn test_lio_onewrite() {
    let wbuf = Rc::new(String::from("abcdef").into_bytes().into_boxed_slice());
    let mut f = tempfile().unwrap();
    let mut rbuf = Vec::new();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(1);
    liocb.emplace_boxed_slice(f.as_raw_fd(), 0, wbuf.clone(), 0,
                              mio_aio::LioOpcode::LIO_WRITE);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);
    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb[0].error().unwrap();
    assert_eq!(liocb[0].aio_return().unwrap(), wbuf.len() as isize);
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read_to_end(&mut rbuf).unwrap();
    assert_eq!(len, wbuf.len());
    assert_eq!(&rbuf.into_boxed_slice(), wbuf.borrow());
}

// Write from a constant buffer
#[test]
pub fn test_lio_onewrite_from_slice() {
    const WBUF: &'static [u8] = b"abcdef";
    let mut f = tempfile().unwrap();
    let mut rbuf = Vec::new();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(1);
    liocb.emplace_slice(f.as_raw_fd(), 0, WBUF, 0,
                        mio_aio::LioOpcode::LIO_WRITE);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);
    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb[0].error().unwrap();
    assert_eq!(liocb[0].aio_return().unwrap(), WBUF.len() as isize);
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read_to_end(&mut rbuf).unwrap();
    assert_eq!(len, WBUF.len());
    assert_eq!(rbuf, WBUF);
}

#[test]
pub fn test_lio_tworeads() {
    const INITIAL: &'static [u8] = b"abcdef123456";
    const EXPECT0: &'static [u8] = b"cdef";
    const EXPECT1: &'static [u8] = b"23456";
    let mut f = tempfile().unwrap();
    let bufs = vec![
        Rc::new(vec![0; 4].into_boxed_slice()),
        Rc::new(vec![0; 5].into_boxed_slice())
    ];
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(2);
    liocb.emplace_boxed_slice(f.as_raw_fd(), 2, bufs[0].clone(), 0,
                              mio_aio::LioOpcode::LIO_READ);
    liocb.emplace_boxed_slice(f.as_raw_fd(), 7, bufs[1].clone(), 0,
                              mio_aio::LioOpcode::LIO_READ);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);

    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb[0].error().unwrap();
    assert_eq!(liocb[0].aio_return().unwrap(), EXPECT0.len() as isize);
    assert!(bufs[0].deref().deref() == EXPECT0);

    liocb[1].error().unwrap();
    assert_eq!(liocb[1].aio_return().unwrap(), EXPECT1.len() as isize);
    assert!(bufs[1].deref().deref() == EXPECT1);
}

#[test]
pub fn test_lio_read_and_write() {
    const INITIAL0: &'static [u8] = b"abcdef123456";
    const WBUF1: &'static [u8] = b"ABCDEFGHIJKL";
    const EXPECT0: &'static [u8] = b"cdef";
    let mut f0 = tempfile().unwrap();
    let mut f1 = tempfile().unwrap();
    let rbuf0 = Rc::new(vec![0; 4].into_boxed_slice());
    let mut rbuf1 = Vec::new();
    f0.write(INITIAL0).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(2);
    liocb.emplace_boxed_slice(f0.as_raw_fd(), 2, rbuf0.clone(), 0,
                              mio_aio::LioOpcode::LIO_READ);
    liocb.emplace_slice(f1.as_raw_fd(), 0, WBUF1, 0,
                        mio_aio::LioOpcode::LIO_WRITE);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);

    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb[0].error().unwrap();
    assert_eq!(liocb[0].aio_return().unwrap(), EXPECT0.len() as isize);
    assert!(rbuf0.deref().deref() == EXPECT0);

    liocb[1].error().unwrap();
    assert_eq!(liocb[1].aio_return().unwrap(), WBUF1.len() as isize);
    f1.seek(SeekFrom::Start(0)).unwrap();
    let len = f1.read_to_end(&mut rbuf1).unwrap();
    assert_eq!(len, WBUF1.len());
    assert_eq!(rbuf1, WBUF1);
}
