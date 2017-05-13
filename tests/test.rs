extern crate libc;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate mio;
extern crate mio_aio;
extern crate nix;
extern crate tempfile;

use mio::{Events, Poll, PollOpt, Token};
use mio::unix::UnixReady;
use tempfile::tempfile;
use nix::sys::aio;
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
        aio::LioOpcode::LIO_NOP);
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

    aiocb.fsync(aio::AioFsyncMode::O_SYNC).unwrap();
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
            aio::LioOpcode::LIO_NOP);
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
        aio::LioOpcode::LIO_NOP);
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

//#[test]
pub fn test_lio_empty() {
    let f = tempfile().unwrap();
    let bufs = vec![];
    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::from_boxed_slices(f.as_raw_fd(),
        0,  //offset
        &bufs,
        0,  //priority
        aio::LioOpcode::LIO_NOP);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);
    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());
}

#[test]
pub fn test_lio_oneread() {
    const INITIAL: &'static [u8] = b"abcdef123456";
    const EXPECT: &'static [u8] = b"cdef";
    let mut f = tempfile().unwrap();
    let bufs = vec![Rc::new(vec![0; 4].into_boxed_slice())];
    //let bufs : [Rc<Box<[u8]>>] = vec![Rc::new(vec![0; 4].into_boxed_slice())];
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::from_boxed_slices(f.as_raw_fd(),
        2,  //offset
        &bufs,
        0,  //priority
        aio::LioOpcode::LIO_READ);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);
    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb.aiocb(0).error().unwrap();
    assert_eq!(liocb.aiocb(0).aio_return().unwrap(), EXPECT.len() as isize);
    assert!(bufs[0].deref().deref() == EXPECT);
}

#[test]
pub fn test_lio_onewrite() {
    let wbuf = Rc::new(String::from("abcdef").into_bytes().into_boxed_slice());
    let mut f = tempfile().unwrap();
    let bufs = vec![wbuf.clone()];
    let mut rbuf = Vec::new();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::from_boxed_slices(f.as_raw_fd(),
        0,  //offset
        &bufs,
        0,  //priority
        aio::LioOpcode::LIO_WRITE);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);
    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb.aiocb(0).error().unwrap();
    assert_eq!(liocb.aiocb(0).aio_return().unwrap(), wbuf.len() as isize);
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read_to_end(&mut rbuf).unwrap();
    assert_eq!(len, wbuf.len());
    assert_eq!(&rbuf.into_boxed_slice(), wbuf.borrow());
}

#[test]
pub fn test_lio_tworeads() {
    const INITIAL: &'static [u8] = b"abcdef123456";
    const EXPECT0: &'static [u8] = b"cdef";
    const EXPECT1: &'static [u8] = b"12345";
    let mut f = tempfile().unwrap();
    let bufs = vec![
        Rc::new(vec![0; 4].into_boxed_slice()),
        Rc::new(vec![0; 5].into_boxed_slice())
    ];
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::from_boxed_slices(f.as_raw_fd(),
        2,  //offset
        &bufs,
        0,  //priority
        aio::LioOpcode::LIO_READ);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);

    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb.aiocb(0).error().unwrap();
    assert_eq!(liocb.aiocb(0).aio_return().unwrap(), EXPECT0.len() as isize);
    assert!(bufs[0].deref().deref() == EXPECT0);

    liocb.aiocb(1).error().unwrap();
    assert_eq!(liocb.aiocb(1).aio_return().unwrap(), EXPECT1.len() as isize);
    println!("bufs[1] == {:?}", bufs[1]);
    assert!(bufs[1].deref().deref() == EXPECT1);
}
