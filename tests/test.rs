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
pub fn test_aio() {
    ::env_logger::init().ok().expect("Couldn't initialize logger");

    debug!("Starting TEST_AIO");
    const WBUF: &'static [u8] = b"abcdef";
    let mut event_loop = EventLoop::<TestHandler>::new().unwrap();
    let mut handler = TestHandler::new();
    let f = tempfile().unwrap();
    let mut aiocb = mio_aio::AioCb::from_slice(f.as_raw_fd(),
        0,   //offset
        &WBUF,
        0,   //priority
        aio::LioOpcode::LIO_NOP);
    debug!("About to register");
    event_loop.register(&aiocb, UDATA, Ready::aio(), PollOpt::empty())
        .ok().expect("registration failed");

    aiocb.write().unwrap();
    event_loop.run_once(&mut handler, None).unwrap();
    assert_eq!(handler.count, 1);
    assert_eq!(handler.last_token, UDATA);

    assert_eq!(aiocb.aio_return().unwrap(), WBUF.len() as isize);
}
