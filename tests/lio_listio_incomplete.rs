// vim: tw=80
extern crate divbuf;
extern crate mio;
extern crate mio_aio;
extern crate nix;
extern crate sysctl;
extern crate tempfile;

use divbuf::{DivBuf, DivBufShared};
use mio::{Events, Poll, PollOpt, Token};
use mio::unix::UnixReady;
use nix::unistd::{SysconfVar, sysconf};
use sysctl::CtlValue;
use std::mem;
use std::os::unix::io::{AsRawFd, RawFd};
use tempfile::tempfile;

fn mk_liocb<'a>(poll: &Poll, token: Token, f: RawFd, num_listios: usize,
            ops_per_listio: u64, i: u64, wbuf: &DivBuf) -> mio_aio::LioCb<'a>
{
    let mut builder = mio_aio::LioCbBuilder::with_capacity(num_listios);
    for j in 0..ops_per_listio {
        let buf = Box::new(wbuf.clone());
        builder = builder.emplace_boxed_slice(
            f,
            4096 * (i * ops_per_listio + j),
            buf,
            0,
            mio_aio::LioOpcode::LIO_WRITE
        );
    }
    let liocb = builder.finish();
    poll.register(&liocb, token, UnixReady::lio().into(), PollOpt::empty())
        .expect("registration failed");
    liocb
}

// An lio_listio(2) call returns EIO.  That means that some of the aiocbs may
// have been initiated, but not all.  This test must run in its own process
// since it intentionally uses all of the system's AIO resources
#[test]
fn lio_listio_incomplete() {
    let alm = sysconf(SysconfVar::AIO_LISTIO_MAX).expect("sysconf").unwrap();
    let maqpp = if let CtlValue::Int(x) = sysctl::value(
            "vfs.aio.max_aio_queue_per_proc").unwrap(){
        x
    } else {
        panic!("unknown sysctl");
    };
    // Find lio_listio sizes that satisfy the AIO_LISTIO_MAX constraint and also
    // result in a final lio_listio call that can only partially be queued
    let mut ops_per_listio = 0;
    let mut num_listios = 0;
    for i in (1..alm).rev() {
        let _ops_per_listio = f64::from(i as u32);
        let _num_listios = (f64::from(maqpp) / _ops_per_listio).ceil();
        let delayed = _ops_per_listio * _num_listios - f64::from(maqpp);
        if delayed > 0.01 {
            ops_per_listio = i as u64;
            num_listios = _num_listios as usize;
            break
        }
    }
    if num_listios == 0 {
        panic!("Can't find a configuration for max_aio_queue_per_proc={} AIO_LISTIO_MAX={}", maqpp, alm);
    }

    let f = tempfile().unwrap();
    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);
    let dbs = DivBufShared::from(vec![0u8; 4096]);
    let wbuf = dbs.try().unwrap();
    let mut liocbs = (0..num_listios).map(|i| {
        Some(mk_liocb(&poll, Token(i), f.as_raw_fd(), num_listios,
                      ops_per_listio, i as u64, &wbuf))
    }).collect::<Vec<_>>();

    let mut submit_results = liocbs
        .iter_mut()
        .map(|liocb| liocb.as_mut().unwrap().submit())
        .collect::<Vec<_>>();
    assert_eq!(submit_results[num_listios - 1],
               Err(mio_aio::LioError::EINCOMPLETE),
               "Testcase didn't produce an incomplete lio_listio");

    let mut complete = 0;
    while complete < num_listios {
        poll.poll(&mut events, None).expect("poll failed");
        for ev in events.iter() {
            assert!(UnixReady::from(ev.readiness()).is_lio());
            let res = match submit_results[ev.token().0] {
                Err(mio_aio::LioError::EINCOMPLETE) => {
                    liocbs[ev.token().0].as_mut().unwrap().resubmit()
                },
                Ok(()) => {
                    let mut liocb = None;
                    mem::swap(&mut liocbs[ev.token().0], &mut liocb);
                    liocb.unwrap().into_results(|iter| {
                        for lr in iter {
                            assert_eq!(lr.result.unwrap(), wbuf.len() as isize);
                        }
                    });
                    complete += 1;
                    Ok(())
                },
                _ => panic!("Unhandled errors")
            };
            submit_results[ev.token().0] = res;
        }
    }
}
