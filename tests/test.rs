extern crate libc;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate mio;
extern crate mio_aio;
extern crate nix;
extern crate tempfile;

use mio::deprecated::{EventLoop, Handler};
use mio::{PollOpt, Ready, Token};
use tempfile::tempfile;
use nix::sys::aio;
use std::os::unix::io::AsRawFd;
use std::io::{Read, Seek, SeekFrom, Write};


const UDATA: Token = Token(0xdeadbeef);

struct TestHandler {
    pub last_token: Token,
    pub count: usize,
}

impl TestHandler {
    fn new() -> TestHandler {
        TestHandler {
            count: 0,
            last_token: Token(0)
        }
    }
}

impl Handler for TestHandler {
    type Timeout = usize;
    type Message = String;

    fn ready(&mut self, _event_loop: &mut EventLoop<TestHandler>, token: Token,
             events: Ready) {
        debug!("READY: {:?} - {:?}", token, events);
        if events.is_aio() {
            debug!("Handler::ready() aio event");
            self.last_token = token;
            self.count += 1;
        }
    }
}

#[test]
pub fn test_cancel() {
    const WBUF: &'static [u8] = b"abcdef";
    let mut event_loop = EventLoop::<TestHandler>::new().unwrap();
    let mut handler = TestHandler::new();
    let f = tempfile().unwrap();
    let mut aiocb = mio_aio::AioCb::from_slice(f.as_raw_fd(),
        0,   //offset
        &WBUF,
        0,   //priority
        aio::LioOpcode::LIO_NOP);
    event_loop.register(&aiocb, UDATA, Ready::aio(), PollOpt::empty())
        .ok().expect("registration failed");

    aiocb.write().unwrap();
    aiocb.cancel().ok().expect("aio_cancel failed");
    event_loop.run_once(&mut handler, None).unwrap();
    assert_eq!(handler.count, 1);
    assert_eq!(handler.last_token, UDATA);

    // Since we cancelled the I/O, we musn't care whether it succeeded.
    let _ = aiocb.aio_return();
}


#[test]
pub fn test_fsync() {
    const INITIAL: &'static [u8] = b"abcdef123456";
    let mut f = tempfile().unwrap();
    f.write(INITIAL).unwrap();
    let mut event_loop = EventLoop::<TestHandler>::new().unwrap();

    let mut handler = TestHandler::new();
    let mut aiocb = mio_aio::AioCb::from_fd( f.as_raw_fd(), 0);
    event_loop.register(&aiocb, UDATA, Ready::aio(), PollOpt::empty())
        .ok().expect("registration failed");

    aiocb.fsync(aio::AioFsyncMode::O_SYNC).unwrap();
    event_loop.run_once(&mut handler, None).unwrap();
    assert_eq!(handler.count, 1);
    assert_eq!(handler.last_token, UDATA);

    aiocb.aio_return().unwrap();
}

#[test]
pub fn test_read() {
    debug!("Starting TEST_AIO");
    const INITIAL: &'static [u8] = b"abcdef123456";
    let mut rbuf = vec![0; 4];
    const EXPECT: &'static [u8] = b"cdef";
    let mut f = tempfile().unwrap();
    f.write(INITIAL).unwrap();

    let mut event_loop = EventLoop::<TestHandler>::new().unwrap();
    let mut handler = TestHandler::new();
    {
        let mut aiocb = mio_aio::AioCb::from_mut_slice(f.as_raw_fd(),
            2,   //offset
            &mut rbuf,
            0,   //priority
            aio::LioOpcode::LIO_NOP);
        event_loop.register(&aiocb, UDATA, Ready::aio(), PollOpt::empty())
            .ok().expect("registration failed");

        aiocb.read().unwrap();
        event_loop.run_once(&mut handler, None).unwrap();
        assert_eq!(handler.count, 1);
        assert_eq!(handler.last_token, UDATA);

        assert_eq!(aiocb.aio_return().unwrap(), EXPECT.len() as isize);
    }
    assert!(rbuf == EXPECT);
}

#[test]
pub fn test_write() {
    debug!("Starting TEST_AIO");
    const WBUF: &'static [u8] = b"abcdef";
    let mut rbuf = Vec::new();
    let mut event_loop = EventLoop::<TestHandler>::new().unwrap();
    let mut handler = TestHandler::new();
    let mut f = tempfile().unwrap();
    let mut aiocb = mio_aio::AioCb::from_slice(f.as_raw_fd(),
        0,   //offset
        &WBUF,
        0,   //priority
        aio::LioOpcode::LIO_NOP);
    event_loop.register(&aiocb, UDATA, Ready::aio(), PollOpt::empty())
        .ok().expect("registration failed");

    aiocb.write().unwrap();
    event_loop.run_once(&mut handler, None).unwrap();
    assert_eq!(handler.count, 1);
    assert_eq!(handler.last_token, UDATA);

    assert_eq!(aiocb.aio_return().unwrap(), WBUF.len() as isize);
    f.seek(SeekFrom::Start(0)).unwrap();
    let len = f.read_to_end(&mut rbuf).unwrap();
    assert!(len == WBUF.len());
    assert!(rbuf == WBUF);
}
