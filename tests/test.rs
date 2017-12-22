extern crate bytes;
extern crate libc;
extern crate env_logger;
extern crate mio;
extern crate mio_aio;
extern crate nix;
extern crate tempfile;

use bytes::{Bytes, BytesMut};
use mio::{Events, Poll, PollOpt, Token};
use mio::unix::UnixReady;
use tempfile::tempfile;
use std::os::unix::io::AsRawFd;
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Deref;


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
    let mut rbuf = vec![0; 4];
    const EXPECT: &'static [u8] = b"cdef";
    let mut f = tempfile().unwrap();
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    {
        let aiocb = mio_aio::AioCb::from_mut_slice(f.as_raw_fd(),
            2,   //offset
            &mut rbuf,
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
pub fn test_aio_read_bytes_small() {
    const INITIAL: &'static [u8] = b"abcdef";
    // rbuf needs to be no more than 32 bytes (64 on 32-bit systems) so
    // BytesMut::clone is implemented by reference.
    let rbuf = BytesMut::from(vec![0; 4]);
    const EXPECT: &'static [u8] = b"cdef";
    let mut f = tempfile().unwrap();
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    let aiocb = mio_aio::AioCb::from_bytes_mut(f.as_raw_fd(),
        2,   //offset
        rbuf,
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
    let buf_ref = aiocb.into_buf_ref();
    assert!(buf_ref.bytes_mut().unwrap() == EXPECT);
}

#[test]
pub fn test_aio_read_bytes_big() {
    const INITIAL: &'static [u8] = b"abcdefgh12345678abcdefgh12345678abcdefgh12345678abcdefgh12345678abcdefgh12345678";
    // rbuf needs to be larger than 32 bytes (64 on 32-bit systems) so
    // BytesMut::clone is implemented by reference.
    let rbuf = BytesMut::from(vec![0; 70]);
    const EXPECT: &'static [u8] = b"cdefgh12345678abcdefgh12345678abcdefgh12345678abcdefgh12345678abcdefgh";
    let mut f = tempfile().unwrap();
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    let aiocb = mio_aio::AioCb::from_bytes_mut(f.as_raw_fd(),
        2,   //offset
        rbuf,
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
    let buf_ref = aiocb.into_buf_ref();
    assert!(buf_ref.bytes_mut().unwrap() == EXPECT);
}

#[test]
pub fn test_aio_write_bytes() {
    let wbuf = Bytes::from(&b"abcdef"[..]);
    let mut f = tempfile().unwrap();
    let mut rbuf = Vec::new();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    let aiocb = mio_aio::AioCb::from_bytes(f.as_raw_fd(),
        0,   //offset
        wbuf.clone(),
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

    assert_eq!(aiocb.aio_return().unwrap(), wbuf.len() as isize);
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read_to_end(&mut rbuf).unwrap();
    assert!(len == wbuf.len());
    assert!(rbuf == wbuf.deref().deref());
    let buf_ref = aiocb.into_buf_ref();
    assert_eq!(buf_ref.bytes().unwrap(), &wbuf.deref());
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
    assert_eq!(events.len(), 1);
    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_aio());

    assert_eq!(aiocb.aio_return().unwrap(), wbuf.len() as isize);
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read_to_end(&mut rbuf).unwrap();
    assert!(len == wbuf.len());
    assert!(rbuf == wbuf.deref().deref());
}

#[test]
pub fn test_aio_write_static() {
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
    let buf = BytesMut::from(vec![0; 4]);
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(1);
    liocb.emplace_bytes_mut(f.as_raw_fd(), 2, buf, 0,
                            mio_aio::LioOpcode::LIO_READ);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);
    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    let mut i = liocb.into_aiocbs();
    let aiocb = i.next().unwrap();
    assert_eq!(aiocb.aio_return().unwrap(), EXPECT.len() as isize);
    assert_eq!(aiocb.into_buf_ref().bytes_mut().unwrap(), EXPECT);
    assert!(i.next().is_none());
}

#[test]
pub fn test_lio_onewrite() {
    let wbuf = Bytes::from(&b"abcdef"[..]);
    let mut f = tempfile().unwrap();
    let mut rbuf = Vec::new();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(1);
    liocb.emplace_bytes(f.as_raw_fd(), 0, wbuf.clone(), 0,
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
    assert_eq!(wbuf, &rbuf);
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
    let rbuf0 = BytesMut::from(vec![0; 4]);
    let rbuf1 = BytesMut::from(vec![0; 5]);
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(2);
    liocb.emplace_bytes_mut(f.as_raw_fd(), 2, rbuf0, 0,
                            mio_aio::LioOpcode::LIO_READ);
    liocb.emplace_bytes_mut(f.as_raw_fd(), 7, rbuf1, 0,
                            mio_aio::LioOpcode::LIO_READ);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);

    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    let mut i = liocb.into_aiocbs();
    let aiocb0 = i.next().unwrap();
    aiocb0.error().unwrap();
    assert_eq!(aiocb0.aio_return().unwrap(), EXPECT0.len() as isize);
    assert_eq!(aiocb0.into_buf_ref().bytes_mut().unwrap(), EXPECT0);

    let aiocb1 = i.next().unwrap();
    aiocb1.error().unwrap();
    assert_eq!(aiocb1.aio_return().unwrap(), EXPECT1.len() as isize);
    assert_eq!(aiocb1.into_buf_ref().bytes_mut().unwrap(), EXPECT1);

    assert!(i.next().is_none());
}

#[test]
pub fn test_lio_read_and_write() {
    const INITIAL0: &'static [u8] = b"abcdef123456";
    const WBUF1: &'static [u8] = b"ABCDEFGHIJKL";
    const EXPECT0: &'static [u8] = b"cdef";
    let mut f0 = tempfile().unwrap();
    let mut f1 = tempfile().unwrap();
    let rbuf0 = BytesMut::from(vec![0; 4]);
    let mut rbuf1 = Vec::new();
    f0.write(INITIAL0).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(2);
    liocb.emplace_bytes_mut(f0.as_raw_fd(), 2, rbuf0, 0,
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

    let mut iter = liocb.into_iter();

    let first = iter.next().unwrap();
    first.error().unwrap();
    assert_eq!(first.aio_return().unwrap(), EXPECT0.len() as isize);
    assert_eq!(first.into_buf_ref().bytes_mut().unwrap(), EXPECT0);

    let second = iter.next().unwrap();
    second.error().unwrap();
    assert_eq!(second.aio_return().unwrap(), WBUF1.len() as isize);
    f1.seek(SeekFrom::Start(0)).unwrap();
    let len = f1.read_to_end(&mut rbuf1).unwrap();
    assert_eq!(len, WBUF1.len());
    assert_eq!(rbuf1, WBUF1);
}

// An lio operation that contains every variant of BufRef.  Tests retrieving the
// BufRefs once the operation is complete.
// Op 1: write from static slice
// Op 2: write from bytes
// Op 3: read into BytesMut
#[test]
pub fn test_lio_buf_ref() {
    const INITIAL: &'static [u8] = b"abcdef123456";
    const WBUF1: &'static [u8] = b"AB";
    let mut rbuf1 = vec![0u8; 2];
    let wbuf2 = Bytes::from(&b"CDEF"[..]);
    let wbuf2clone = wbuf2.clone();
    let mut rbuf2 = vec![0u8; 4];
    let rbuf3 = BytesMut::from(vec![0; 6]);
    const EXPECT3: &'static [u8] = b"123456";
    let mut f = tempfile().unwrap();
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(3);
    liocb.emplace_slice(f.as_raw_fd(), 0, WBUF1, 0,
                        mio_aio::LioOpcode::LIO_WRITE);
    liocb.emplace_bytes(f.as_raw_fd(), 2, wbuf2, 0,
                              mio_aio::LioOpcode::LIO_WRITE);
    liocb.emplace_bytes_mut(f.as_raw_fd(), 6, rbuf3, 0,
                              mio_aio::LioOpcode::LIO_READ);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.listio().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    assert_eq!(events.len(), 1);

    let ev = events.get(0).unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    let mut iter = liocb.into_iter();

    let first = iter.next().unwrap();
    first.error().unwrap();
    assert_eq!(first.aio_return().unwrap(), WBUF1.len() as isize);
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read(&mut rbuf1).unwrap();
    assert_eq!(len, WBUF1.len());
    assert_eq!(rbuf1, WBUF1);
    let buf_ref = first.into_buf_ref();
    assert!(buf_ref.is_none());

    let second = iter.next().unwrap();
    second.error().unwrap();
    assert_eq!(second.aio_return().unwrap(), wbuf2clone.len() as isize);
    f.seek(SeekFrom::Start(2)).unwrap();
    let len = f.read(&mut rbuf2).unwrap();
    assert_eq!(len, wbuf2clone.len());
    assert_eq!(rbuf2, wbuf2clone.to_vec());
    let buf_ref = second.into_buf_ref();
    assert_eq!(buf_ref.bytes().unwrap(), &wbuf2clone.deref());

    let third = iter.next().unwrap();
    third.error().unwrap();
    assert_eq!(third.aio_return().unwrap(), EXPECT3.len() as isize);
    let buf_ref = third.into_buf_ref();
    assert_eq!(buf_ref.bytes_mut().unwrap(), &EXPECT3);
}
