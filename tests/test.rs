extern crate mio;
extern crate mio_aio;
extern crate tempfile;

use mio::{Events, Interest, Poll, Token};
use mio_aio::SourceApi;
use tempfile::tempfile;
use std::os::unix::io::AsRawFd;
use std::io::{IoSlice, IoSliceMut, Read, Seek, SeekFrom, Write};
use std::ops::Deref;


const UDATA: Token = Token(0xdead_beef);

#[test]
pub fn test_aio_cancel() {
    const WBUF: &[u8] = b"abcdef";
    let f = tempfile().unwrap();

    let mut poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    let mut aiocb = mio_aio::WriteAt::write_at(f.as_raw_fd(),
        0,   //offset
        WBUF,
        0,   //priority
    );
    poll.registry().register(&mut aiocb, UDATA, Interest::AIO)
        .expect("registration failed");
    let mut aiocb = Box::pin(aiocb);

    aiocb.as_mut().submit().unwrap();
    aiocb.as_mut().cancel().expect("aio_cancel failed");

    poll.poll(&mut events, None).expect("poll failed");
    let mut it = events.iter();
    let ev = it.next().unwrap();
    assert_eq!(ev.token(), UDATA);
    assert!(ev.is_aio());

    // Since we cancelled the I/O, we musn't care whether it succeeded.
    let _ = aiocb.as_mut().aio_return();
    assert!(it.next().is_none());
}

mod aio_fsync {
    use super::*;

    #[test]
    fn ok() {
        const INITIAL: &[u8] = b"abcdef123456";
        let mut f = tempfile().unwrap();
        f.write_all(INITIAL).unwrap();
        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(1024);

        let mut aiof = mio_aio::Source::fsync(
            f.as_raw_fd(),
            mio_aio::AioFsyncMode::O_SYNC,
            0
        );
        poll.registry().register(&mut aiof, UDATA, Interest::AIO)
            .expect("registration failed");

        let mut aiof = Box::pin(aiof);
        aiof.as_mut().submit().unwrap();
        poll.poll(&mut events, None).expect("poll failed");
        let mut it = events.iter();
        let ev = it.next().unwrap();
        assert_eq!(ev.token(), UDATA);
        assert!(ev.is_aio());

        assert!(aiof.as_mut().error().is_ok());
        aiof.as_mut().aio_return().unwrap();
        assert!(it.next().is_none());
    }

}

mod aio_read {
    use super::*;

    #[test]
    fn ok() {
        const INITIAL: &[u8] = b"abcdef123456";
        let mut rbuf = vec![0; 4];
        const EXPECT: &[u8] = b"cdef";
        let mut f = tempfile().unwrap();
        f.write_all(INITIAL).unwrap();

        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(1024);
        {
            let mut aior = mio_aio::Source::read_at(f.as_raw_fd(),
                2,   //offset
                &mut rbuf,
                0,   //priority
            );
            poll.registry().register(&mut aior, UDATA, Interest::AIO)
                .expect("registration failed");
            let mut aior = Box::pin(aior);

            aior.as_mut().submit().unwrap();

            poll.poll(&mut events, None).expect("poll failed");
            let mut it = events.iter();
            let ev = it.next().unwrap();
            assert_eq!(ev.token(), UDATA);
            assert!(ev.is_aio());

            assert!(aior.as_mut().error().is_ok());
            assert_eq!(aior.as_mut().aio_return().unwrap(), EXPECT.len());
            assert!(it.next().is_none());
        }
        assert!(rbuf.deref().deref() == EXPECT);
    }
}

mod aio_readv {
    use super::*;

    #[test]
    fn ok() {
        const INITIAL: &[u8] = b"abcdef123456";
        let mut rbuf0 = vec![0; 4];
        let mut rbuf1 = vec![0; 2];
        let mut rbufs = [IoSliceMut::new(&mut rbuf0),
            IoSliceMut::new(&mut rbuf1)];
        const EXPECT0: &[u8] = b"cdef";
        const EXPECT1: &[u8] = b"12";
        let mut f = tempfile().unwrap();
        f.write_all(INITIAL).unwrap();

        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(1024);
        {
            let mut aior = mio_aio::Source::readv_at(f.as_raw_fd(),
                2,   //offset
                &mut rbufs,
                0,   //priority
            );
            poll.registry().register(&mut aior, UDATA, Interest::AIO)
                .expect("registration failed");
            let mut aior = Box::pin(aior);

            aior.as_mut().submit().unwrap();

            poll.poll(&mut events, None).expect("poll failed");
            let mut it = events.iter();
            let ev = it.next().unwrap();
            assert_eq!(ev.token(), UDATA);
            assert!(ev.is_aio());

            assert!(aior.as_mut().error().is_ok());
            assert_eq!(aior.as_mut().aio_return().unwrap(),
                       (EXPECT0.len() + EXPECT1.len()));
            assert!(it.next().is_none());
        }
        assert!(rbuf0 == EXPECT0);
        assert!(rbuf1 == EXPECT1);
    }
}

mod aio_write {
    use super::*;

    #[test]
    fn cancel() {
        let wbuf = String::from("abcdef").into_bytes().into_boxed_slice();
        let f = tempfile().unwrap();

        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(1024);
        let mut aiow = mio_aio::Source::write_at(f.as_raw_fd(), 0, &wbuf, 0);
        poll.registry().register(&mut aiow, UDATA, Interest::AIO)
            .expect("registration failed");
        let mut aiow = Box::pin(aiow);

        aiow.as_mut().submit().unwrap();
        aiow.as_mut().cancel().unwrap();

        poll.poll(&mut events, None).expect("poll failed");
        let mut it = events.iter();
        let ev = it.next().unwrap();
        assert_eq!(ev.token(), UDATA);
        assert!(ev.is_aio());

        // Since we cancelled the I/O, we musn't care whether it succeeded.
        let _ = aiow.as_mut().aio_return();
        assert!(it.next().is_none());
    }

    #[test]
    fn ok() {
        let wbuf = String::from("abcdef").into_bytes().into_boxed_slice();
        let mut f = tempfile().unwrap();
        let mut rbuf = Vec::new();

        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(1024);
        let mut aiow = mio_aio::Source::write_at(f.as_raw_fd(), 0, &wbuf, 0);
        poll.registry().register(&mut aiow, UDATA, Interest::AIO)
            .expect("registration failed");
        let mut aiow = Box::pin(aiow);

        aiow.as_mut().submit().unwrap();

        poll.poll(&mut events, None).expect("poll failed");
        let mut it = events.iter();
        let ev = it.next().unwrap();
        assert_eq!(ev.token(), UDATA);
        assert!(ev.is_aio());

        assert!(aiow.as_mut().error().is_ok());
        assert_eq!(aiow.as_mut().aio_return().unwrap(), wbuf.len());
        f.seek(SeekFrom::Start(0)).unwrap();
        let len = f.read_to_end(&mut rbuf).unwrap();
        assert!(len == wbuf.len());
        assert!(rbuf == wbuf.deref().deref());
        assert!(it.next().is_none());
    }
}

mod aio_writev {
    use super::*;

    #[test]
    fn ok() {
        let wbuf0 = b"abcde";
        let wbuf1 = b"fghi";
        let wbufs = [IoSlice::new(wbuf0), IoSlice::new(wbuf1)];
        let expected = b"abcdefghi";
        let mut f = tempfile().unwrap();
        let mut rbuf = Vec::new();

        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(1024);
        let mut aiow = mio_aio::Source::writev_at(f.as_raw_fd(), 0, &wbufs, 0);
        poll.registry().register(&mut aiow, UDATA, Interest::AIO)
            .expect("registration failed");
        let mut aiow = Box::pin(aiow);

        aiow.as_mut().submit().unwrap();

        poll.poll(&mut events, None).expect("poll failed");
        let mut it = events.iter();
        let ev = it.next().unwrap();
        assert_eq!(ev.token(), UDATA);
        assert!(ev.is_aio());

        assert!(aiow.as_mut().error().is_ok());
        assert_eq!(aiow.as_mut().aio_return().unwrap(), expected.len());
        f.seek(SeekFrom::Start(0)).unwrap();
        let len = f.read_to_end(&mut rbuf).unwrap();
        assert_eq!(len, expected.len());
        assert_eq!(expected, &rbuf[..]);
        assert!(it.next().is_none());
    }
}
