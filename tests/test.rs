extern crate divbuf;
extern crate mio;
extern crate mio_aio;
extern crate tempfile;

use divbuf::DivBufShared;
use mio::{Events, Poll, PollOpt, Token};
use mio::unix::UnixReady;
use tempfile::tempfile;
use std::os::unix::io::AsRawFd;
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Deref;


const UDATA: Token = Token(0xdeadbeef);

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
    aiocb.cancel().ok().expect("aio_cancel failed");

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
    f.write(INITIAL).unwrap();
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
pub fn test_aio_read_divbuf() {
    const INITIAL: &[u8] = b"abcdef";
    let dbs = DivBufShared::from(vec![0u8; 4]);
    let rbuf = Box::new(dbs.try_mut().unwrap());
    const EXPECT: &[u8] = b"cdef";
    let mut f = tempfile().unwrap();
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    let mut aiocb = mio_aio::AioCb::from_boxed_mut_slice(f.as_raw_fd(),
        2,   //offset
        rbuf,
        0,   //priority
        mio_aio::LioOpcode::LIO_NOP);
    poll.register(&aiocb, UDATA, UnixReady::aio().into(), PollOpt::empty())
        .ok().expect("registration failed");

    aiocb.read().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_aio());

    assert!(aiocb.error().is_ok());
    assert_eq!(aiocb.aio_return().unwrap(), EXPECT.len() as isize);
    let mut buf_ref = aiocb.buf_ref();
    assert_eq!(buf_ref.boxed_mut_slice().unwrap().borrow(), EXPECT);
    assert!(it.next().is_none());
}

#[test]
pub fn test_aio_write_divbuf() {
    let dbs = DivBufShared::from(&b"abcdef"[..]);
    let wbuf = Box::new(dbs.try().unwrap());
    let mut f = tempfile().unwrap();
    let mut rbuf = Vec::new();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    let mut aiocb = mio_aio::AioCb::from_boxed_slice(f.as_raw_fd(),
        0,   //offset
        wbuf.clone(),
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
    let buf_ref = aiocb.buf_ref();
    assert_eq!(buf_ref.boxed_slice().unwrap().borrow(), &wbuf[..]);
    assert!(it.next().is_none());
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
    let dbs = DivBufShared::from(&b"abcdef"[..]);
    let wbuf0 = Box::new(dbs.try().unwrap());
    let poll = Poll::new().unwrap();

    let mut liocb = mio_aio::LioCb::with_capacity(1);
    let fd = -1;    // Illegal file descriptor
    liocb.emplace_boxed_slice(fd, 0, wbuf0, 0,
                        mio_aio::LioOpcode::LIO_WRITE);
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
    let dbs = DivBufShared::from(vec![0; 4]);
    let buf = Box::new(dbs.try_mut().unwrap());
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(1);
    liocb.emplace_boxed_mut_slice(f.as_raw_fd(), 2, buf, 0,
        mio_aio::LioOpcode::LIO_READ);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.submit().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb.into_results(|mut iter| {
        let mut lr = iter.next().unwrap();
        assert_eq!(lr.result.unwrap(), EXPECT.len() as isize);
        assert_eq!(lr.buf_ref.boxed_mut_slice().unwrap().borrow(), EXPECT);
        assert!(iter.next().is_none());
    });
    assert!(it.next().is_none());
}

#[test]
pub fn test_lio_onewrite() {
    let dbs = DivBufShared::from(&b"abcdef"[..]);
    let wbuf0 = Box::new(dbs.try().unwrap());
    let mut f = tempfile().unwrap();
    let mut rbuf = Vec::new();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(1);
    liocb.emplace_boxed_slice(f.as_raw_fd(), 0, wbuf0, 0,
                        mio_aio::LioOpcode::LIO_WRITE);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.submit().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    let wbuf1 = dbs.try().unwrap();
    liocb.into_results(|mut iter| {
        let lr = iter.next().unwrap();
        assert_eq!(lr.result.unwrap(), wbuf1.len() as isize);
        assert!(iter.next().is_none());
    });
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read_to_end(&mut rbuf).unwrap();
    assert_eq!(len, wbuf1.len());
    assert_eq!(&wbuf1[..], &rbuf[..]);
    assert!(it.next().is_none());
}

// Write from a constant buffer
#[test]
pub fn test_lio_onewrite_from_slice() {
    const WBUF: &[u8] = b"abcdef";
    let mut f = tempfile().unwrap();
    let mut rbuf = Vec::new();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(1);
    liocb.emplace_slice(f.as_raw_fd(), 0, WBUF, 0,
                        mio_aio::LioOpcode::LIO_WRITE);
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
    let dbs0 = DivBufShared::from(vec![0; 4]);
    let rbuf0 = Box::new(dbs0.try_mut().unwrap());
    let dbs1 = DivBufShared::from(vec![0; 5]);
    let rbuf1 = Box::new(dbs1.try_mut().unwrap());
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(2);
    liocb.emplace_boxed_mut_slice(f.as_raw_fd(), 2, rbuf0, 0,
                            mio_aio::LioOpcode::LIO_READ);
    liocb.emplace_boxed_mut_slice(f.as_raw_fd(), 7, rbuf1, 0,
                            mio_aio::LioOpcode::LIO_READ);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.submit().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb.into_results(|mut iter| {
        let mut lr0 = iter.next().unwrap();
        assert_eq!(lr0.result.unwrap(), EXPECT0.len() as isize);
        assert_eq!(lr0.buf_ref.boxed_mut_slice().unwrap().borrow(), EXPECT0);
        let mut lr1 = iter.next().unwrap();
        assert_eq!(lr1.result.unwrap(), EXPECT1.len() as isize);
        assert_eq!(lr1.buf_ref.boxed_mut_slice().unwrap().borrow(), EXPECT1);
        assert!(iter.next().is_none());
    });

    assert!(it.next().is_none());
}

#[test]
pub fn test_lio_read_and_write() {
    const INITIAL0: &[u8] = b"abcdef123456";
    const WBUF1: &[u8] = b"ABCDEFGHIJKL";
    const EXPECT0: &[u8] = b"cdef";
    let mut f0 = tempfile().unwrap();
    let mut f1 = tempfile().unwrap();
    let dbs0 = DivBufShared::from(vec![0; 4]);
    let rbuf0 = Box::new(dbs0.try_mut().unwrap());
    let mut rbuf1 = Vec::new();
    f0.write(INITIAL0).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(2);
    liocb.emplace_boxed_mut_slice(f0.as_raw_fd(), 2, rbuf0, 0,
                            mio_aio::LioOpcode::LIO_READ);
    liocb.emplace_slice(f1.as_raw_fd(), 0, WBUF1, 0,
                        mio_aio::LioOpcode::LIO_WRITE);
    poll.register(&liocb, UDATA, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");

    liocb.submit().unwrap();

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(UnixReady::from(ev.readiness()).is_lio());

    liocb.into_results(|mut iter| {
        let mut lr0 = iter.next().unwrap();
        assert_eq!(lr0.result.unwrap(), EXPECT0.len() as isize);
        assert_eq!(lr0.buf_ref.boxed_mut_slice().unwrap().borrow(), EXPECT0);

        let lr1 = iter.next().unwrap();
        assert_eq!(lr1.result.unwrap(), WBUF1.len() as isize);

        assert!(iter.next().is_none());
    });

    f1.seek(SeekFrom::Start(0)).unwrap();
    let len = f1.read_to_end(&mut rbuf1).unwrap();
    assert_eq!(len, WBUF1.len());
    assert_eq!(rbuf1, WBUF1);

    assert!(it.next().is_none());
}

// An lio operation that contains every variant of BufRef.  Tests retrieving the
// BufRefs once the operation is complete.
// Op 1: write from static slice
// Op 4: write from BoxedSlice
// Op 5: read into BoxedMutSlice
#[test]
pub fn test_lio_buf_ref() {
    const INITIAL: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    const WBUF1: &[u8] = b"AB";
    let mut rbuf1 = vec![0u8; 2];
    let dbs4 = DivBufShared::from(&b"QXYZ"[..]);
    let db4 = Box::new(dbs4.try().unwrap());
    let mut rbuf4 = vec![0u8; 4];
    let dbs5 = DivBufShared::from(vec![0; 8]);
    let dbm5 = Box::new(dbs5.try_mut().unwrap());
    const EXPECT5: &[u8] = b"qrstuvwx";
    let mut f = tempfile().unwrap();
    f.write(INITIAL).unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    let mut liocb = mio_aio::LioCb::with_capacity(3);
    liocb.emplace_slice(f.as_raw_fd(), 0, WBUF1, 0,
                        mio_aio::LioOpcode::LIO_WRITE);
    liocb.emplace_boxed_slice(f.as_raw_fd(), 6, db4, 0,
                              mio_aio::LioOpcode::LIO_WRITE);
    liocb.emplace_boxed_mut_slice(f.as_raw_fd(), 16, dbm5, 0,
                              mio_aio::LioOpcode::LIO_READ);
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
        f.seek(SeekFrom::Start(0)).unwrap();
        let len = f.read(&mut rbuf1).unwrap();
        assert_eq!(len, WBUF1.len());
        assert_eq!(rbuf1, WBUF1);
        assert_eq!(lr0.result.unwrap(), WBUF1.len() as isize);
        assert!(lr0.buf_ref.is_none());

        let lr1 = iter.next().unwrap();
        assert_eq!(lr1.result.unwrap(), dbs4.len() as isize);
        f.seek(SeekFrom::Start(6)).unwrap();
        let len = f.read(&mut rbuf4).unwrap();
        assert_eq!(len, dbs4.len());
        assert_eq!(rbuf4[..], dbs4.try().unwrap()[..]);
        assert_eq!(lr1.buf_ref.boxed_slice().unwrap().borrow(), &rbuf4[..]);

        let mut lr2 = iter.next().unwrap();
        assert_eq!(lr2.result.unwrap(), EXPECT5.len() as isize);
        assert_eq!(lr2.buf_ref.boxed_mut_slice().unwrap().borrow(), &EXPECT5[..]);

        assert!(iter.next().is_none());
    });

    assert!(it.next().is_none());
}
