extern crate mio;
extern crate mio_aio;
extern crate tempfile;

use mio::{Events, Poll, PollOpt, Token};
use mio::unix::UnixReady;
use tempfile::tempfile;
use std::os::unix::io::AsRawFd;
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Deref;


const UDATA: Token = Token(0xdead_beef);

#[test]
pub fn test_aio_cancel() {
    const WBUF: &[u8] = b"abcdef";
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
    aiocb.cancel().expect("aio_cancel failed");

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_aio());

    // Since we cancelled the I/O, we musn't care whether it succeeded.
    let _ = aiocb.aio_return();
    assert!(it.next().is_none());
}


#[test]
pub fn test_aio_fsync() {
    const INITIAL: &[u8] = b"abcdef123456";
    let mut f = tempfile().unwrap();
    f.write_all(INITIAL).unwrap();
    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let aiocb = mio_aio::AioCb::from_fd( f.as_raw_fd(), 0);
    poll.register(&aiocb, UDATA, UnixReady::aio().into(), PollOpt::empty())
        .expect("registration failed");

    aiocb.fsync(mio_aio::AioFsyncMode::O_SYNC).unwrap();
    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_aio());

    assert!(aiocb.error().is_ok());
    aiocb.aio_return().unwrap();
    assert!(it.next().is_none());
}

#[test]
pub fn test_aio_read() {
    const INITIAL: &[u8] = b"abcdef123456";
    let mut rbuf = vec![0; 4];
    const EXPECT: &[u8] = b"cdef";
    let mut f = tempfile().unwrap();
    f.write_all(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    {
        let aiocb = mio_aio::AioCb::from_mut_slice(f.as_raw_fd(),
            2,   //offset
            &mut rbuf,
            0,   //priority
            mio_aio::LioOpcode::LIO_NOP);
        poll.register(&aiocb, UDATA, UnixReady::aio().into(), PollOpt::empty())
            .expect("registration failed");

        aiocb.read().unwrap();

        poll.poll(&mut events, None).expect("poll failed");
        let mut it = events.iter();
        let ev = it.next().unwrap();
        assert_eq!(ev.token(), UDATA);
        assert!(UnixReady::from(ev.readiness()).is_aio());

        assert!(aiocb.error().is_ok());
        assert_eq!(aiocb.aio_return().unwrap(), EXPECT.len() as isize);
        assert!(it.next().is_none());
    }
    assert!(rbuf.deref().deref() == EXPECT);
}

#[test]
pub fn test_aio_write_slice() {
    let wbuf = String::from("abcdef").into_bytes().into_boxed_slice();
    let mut f = tempfile().unwrap();
    let mut rbuf = Vec::new();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    let aiocb = mio_aio::AioCb::from_slice(f.as_raw_fd(),
        0,   //offset
        &wbuf,
        0,   //priority
        mio_aio::LioOpcode::LIO_NOP);
    poll.register(&aiocb, UDATA, UnixReady::aio().into(), PollOpt::empty())
        .expect("registration failed");

    aiocb.write().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_aio());

    assert!(aiocb.error().is_ok());
    assert_eq!(aiocb.aio_return().unwrap(), wbuf.len() as isize);
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read_to_end(&mut rbuf).unwrap();
    assert!(len == wbuf.len());
    assert!(rbuf == wbuf.deref().deref());
    assert!(it.next().is_none());
}

#[test]
pub fn test_aio_write_static() {
    const WBUF: &[u8] = b"abcdef";
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
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_aio());

    assert!(aiocb.error().is_ok());
    assert_eq!(aiocb.aio_return().unwrap(), WBUF.len() as isize);
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read_to_end(&mut rbuf).unwrap();
    assert!(len == WBUF.len());
    assert!(rbuf == WBUF);
    assert!(it.next().is_none());
}

// lio_listio returns EIO because one of its children failed
#[test]
pub fn test_lio_eio() {
    let wbuf0 = &b"abcdef"[..];
    let poll = Poll::new().unwrap();

    let fd = -1;    // Illegal file descriptor
    let mut liocb = mio_aio::LioCbBuilder::with_capacity(1)
        .emplace_slice(
            fd,
            0,
            wbuf0,
            0,
            mio_aio::LioOpcode::LIO_WRITE
        ).finish();
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    let r = liocb.submit();
    let expected = Err(mio_aio::Errno::EBADF);
    assert_eq!(r.unwrap_err().into_eio().unwrap()[0], expected);
}

#[test]
pub fn test_lio_oneread() {
    const INITIAL: &[u8] = b"abcdef123456";
    const EXPECT: &[u8] = b"cdef";
    let mut f = tempfile().unwrap();
    let mut buf = vec![0; 4];
    f.write_all(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCbBuilder::with_capacity(1)
        .emplace_mut_slice(
            f.as_raw_fd(),
            2,
            &mut buf[..],
            0,
            mio_aio::LioOpcode::LIO_READ
        ).finish();
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.submit().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb.into_results(|mut iter| {
        let lr = iter.next().unwrap();
        assert_eq!(lr.result.unwrap(), EXPECT.len() as isize);
        assert!(iter.next().is_none());
    });
    assert!(it.next().is_none());
    assert_eq!(&buf[..], EXPECT);
}

// Write from a constant buffer
#[test]
pub fn test_lio_onewrite_from_slice() {
    const WBUF: &[u8] = b"abcdef";
    let mut f = tempfile().unwrap();
    let mut rbuf = Vec::new();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCbBuilder::with_capacity(1)
        .emplace_slice(
            f.as_raw_fd(),
            0,
            WBUF,
            0,
            mio_aio::LioOpcode::LIO_WRITE
        ).finish();
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.submit().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb.into_results(|mut iter| {
        let lr = iter.next().unwrap();
        assert_eq!(lr.result.unwrap(), WBUF.len() as isize);
        assert!(iter.next().is_none());
    });
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read_to_end(&mut rbuf).unwrap();
    assert_eq!(len, WBUF.len());
    assert_eq!(rbuf, WBUF);
    assert!(it.next().is_none());
}

#[test]
pub fn test_lio_tworeads() {
    const INITIAL: &[u8] = b"abcdef123456";
    const EXPECT0: &[u8] = b"cdef";
    const EXPECT1: &[u8] = b"23456";
    let mut f = tempfile().unwrap();
    let mut rbuf0 = vec![0; 4];
    let mut rbuf1 = vec![0; 5];
    f.write_all(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCbBuilder::with_capacity(2)
        .emplace_mut_slice(
            f.as_raw_fd(),
            2,
            &mut rbuf0[..],
            0,
            mio_aio::LioOpcode::LIO_READ
        ).emplace_mut_slice(
            f.as_raw_fd(),
            7,
            &mut rbuf1[..],
            0,
            mio_aio::LioOpcode::LIO_READ
        ).finish();
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.submit().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb.into_results(|mut iter| {
        let lr0 = iter.next().unwrap();
        assert_eq!(lr0.result.unwrap(), EXPECT0.len() as isize);
        let lr1 = iter.next().unwrap();
        assert_eq!(lr1.result.unwrap(), EXPECT1.len() as isize);
        assert!(iter.next().is_none());
    });
    assert!(it.next().is_none());
    assert_eq!(&rbuf0[..], EXPECT0);
    assert_eq!(&rbuf1[..], EXPECT1);
}

#[test]
pub fn test_lio_read_and_write() {
    const INITIAL0: &[u8] = b"abcdef123456";
    const WBUF1: &[u8] = b"ABCDEFGHIJKL";
    const EXPECT0: &[u8] = b"cdef";
    let mut f0 = tempfile().unwrap();
    let mut f1 = tempfile().unwrap();
    let mut rbuf0 = vec![0; 4];
    let mut rbuf1 = Vec::new();
    f0.write_all(INITIAL0).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCbBuilder::with_capacity(2)
        .emplace_mut_slice(
            f0.as_raw_fd(),
            2,
            &mut rbuf0[..],
            0,
            mio_aio::LioOpcode::LIO_READ
        ).emplace_slice(
            f1.as_raw_fd(),
            0,
            WBUF1,
            0,
            mio_aio::LioOpcode::LIO_WRITE
        ).finish();
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.submit().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb.into_results(|mut iter| {
        let lr0 = iter.next().unwrap();
        assert_eq!(lr0.result.unwrap(), EXPECT0.len() as isize);

        let lr1 = iter.next().unwrap();
        assert_eq!(lr1.result.unwrap(), WBUF1.len() as isize);

        assert!(iter.next().is_none());
    });

    assert_eq!(&rbuf0[..], EXPECT0);
    f1.seek(SeekFrom::Start(0)).unwrap();
    let len = f1.read_to_end(&mut rbuf1).unwrap();
    assert_eq!(len, WBUF1.len());
    assert_eq!(rbuf1, WBUF1);

    assert!(it.next().is_none());
}
