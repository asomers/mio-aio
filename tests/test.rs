extern crate libc;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate mio;
extern crate mio_aio;
extern crate nix;
extern crate tempfile;

use mio::{Events, Poll, PollOpt, Ready, Token};
use mio::unix::UnixReady;
use tempfile::tempfile;
use nix::sys::aio;
use std::os::unix::io::AsRawFd;
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Deref;
use std::rc::Rc;


const UDATA: Token = Token(0xdeadbeef);

#[test]
pub fn test_cancel() {
    const WBUF: &'static [u8] = b"abcdef";
    let f = tempfile().unwrap();

    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    let aiocb = mio_aio::AioCb::from_slice(f.as_raw_fd(),
        0,   //offset
        &WBUF,
        0,   //priority
        aio::LioOpcode::LIO_NOP);
    poll.register(&aiocb, UDATA, Ready::from(UnixReady::aio()), PollOpt::empty())
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
pub fn test_fsync() {
    const INITIAL: &'static [u8] = b"abcdef123456";
    let mut f = tempfile().unwrap();
    f.write(INITIAL).unwrap();
    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    //let mut handler = TestHandler::new();
    let aiocb = mio_aio::AioCb::from_fd( f.as_raw_fd(), 0);
    poll.register(&aiocb, UDATA, Ready::from(UnixReady::aio()), PollOpt::empty())
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
pub fn test_read() {
    debug!("Starting TEST_AIO");
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
        poll.register(&aiocb, UDATA, Ready::from(UnixReady::aio()), PollOpt::empty())
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
pub fn test_write() {
    debug!("Starting TEST_AIO");
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
    poll.register(&aiocb, UDATA, Ready::from(UnixReady::aio()), PollOpt::empty())
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
