// vim: tw=80
use nix::libc::{c_int, off_t};
use mio::{Evented, Poll, Token, Ready, PollOpt};
use mio::unix::UnixReady;
use nix::errno::Errno;
use nix::sys::aio;
use nix::sys::signal::SigevNotify;
use std::io;
use std::iter::Iterator;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::RawFd;
use std::pin::Pin;
use std::sync::Mutex;

pub use nix::sys::aio::AioFsyncMode;
pub use nix::sys::aio::LioOpcode;


/// Represents the result of an individual operation from an `LioCb::submit`
/// call.
pub struct LioResult {
    pub result: nix::Result<isize>
}

// LCOV_EXCL_START
#[derive(Debug)]
pub struct AioCb<'a> {
    // Must use Pin for the AioCb so its location in memory will be
    // constant.  It is an error to move a libc::aiocb after passing it to the
    // kernel.
    // Must use a Mutex because mio::Evented::register takes an immutable
    // receiver, but we need to modify the libc::aiocb.  I think the Mutex will
    // be unnecessary with mio-0.7.
    inner: Mutex<Pin<Box<aio::AioCb<'a>>>>,
}
// LCOV_EXCL_STOP

/// Wrapper around nix::sys::aio::AioCb.
///
/// Implements mio::Evented.  After creation, use mio::Evented::register to
/// connect to the event loop
impl<'a> AioCb<'a> {
    /// Wraps nix::sys::aio::AioCb::from_fd.
    pub fn from_fd(fd: RawFd, prio: c_int) -> AioCb<'a> {
        let aiocb = aio::AioCb::from_fd(fd, prio, SigevNotify::SigevNone);
        AioCb { inner: Mutex::new(aiocb) }
    }

    /// Wraps nix::sys::aio::from_mut_slice
    ///
    /// Not as useful as it sounds, because in typical mio use cases, the
    /// compiler can't guarantee that the slice's lifetime is respected.
    pub fn from_mut_slice(fd: RawFd, offs: u64, buf: &'a mut [u8],
                      prio: c_int, opcode: LioOpcode) -> AioCb {
        let aiocb = aio::AioCb::from_mut_slice(fd, offs as off_t, buf, prio,
                                           SigevNotify::SigevNone, opcode);
        AioCb { inner: Mutex::new(aiocb) }
    }

    /// Wraps nix::sys::aio::from_slice
    ///
    /// Mostly useful for writing constant slices
    pub fn from_slice(fd: RawFd, offs: u64, buf: &'a [u8],
                      prio: c_int, opcode: LioOpcode) -> AioCb {
        let aiocb = aio::AioCb::from_slice(fd, offs as off_t, buf, prio,
                                           SigevNotify::SigevNone, opcode);
        AioCb { inner: Mutex::new(aiocb) }
    }

    /// Read the final result of the operation
    pub fn aio_return(&mut self) -> nix::Result<isize> {
        self.inner.get_mut().unwrap().aio_return()
    }   // LCOV_EXCL_LINE

    /// Ask the operating system to cancel the operation
    ///
    /// Most file systems on most operating systems don't actually support
    /// cancellation; they'll just return `AIO_NOTCANCELED`.
    pub fn cancel(&mut self) -> nix::Result<aio::AioCancelStat> {
        self.inner.get_mut().unwrap().cancel()
    }   // LCOV_EXCL_LINE

    /// Retrieve the status of an in-progress or complete operation.
    ///
    /// Not usually needed, since `mio_aio` always uses kqueue for notification.
    pub fn error(&mut self) -> nix::Result<()> {
        self.inner.get_mut().unwrap().error()
    }   // LCOV_EXCL_LINE

    /// Asynchronously fsync a file.
    pub fn fsync(&mut self, mode: AioFsyncMode) -> nix::Result<()> {
        self.inner.get_mut().unwrap().fsync(mode)
    }   // LCOV_EXCL_LINE

    /// Asynchronously read from a file.
    pub fn read(&mut self) -> nix::Result<()> {
        self.inner.get_mut().unwrap().read()
    }   // LCOV_EXCL_LINE

    /// Asynchronously write to a file.
    pub fn write(&mut self) -> nix::Result<()> {
        self.inner.get_mut().unwrap().write()
    }   // LCOV_EXCL_LINE
}

impl<'a> Evented for AioCb<'a> {
    fn register(&self,
                poll: &Poll,
                token: Token,
                events: Ready,
                _: PollOpt) -> io::Result<()> {
        assert!(UnixReady::from(events).is_aio());
        let udata = usize::from(token);
        let kq = poll.as_raw_fd();
        let sigev = SigevNotify::SigevKevent{kq, udata: udata as isize};
        self.inner.lock().unwrap().set_sigev_notify(sigev);
        Ok(())
    }

    fn reregister(&self,
                  poll: &Poll,
                  token: Token,
                  events: Ready,
                  opts: PollOpt) -> io::Result<()> {
        self.register(poll, token, events, opts)
    }

    fn deregister(&self, _: &Poll) -> io::Result<()> {
        let sigev = SigevNotify::SigevNone;
        self.inner.lock().unwrap().set_sigev_notify(sigev);
        Ok(())
    }
}


// LCOV_EXCL_START
#[derive(Debug)]
pub struct LioCb<'a> {
    // Unlike AioCb, registering this structure does not modify the AioCb's
    // themselves, so no Mutex is needed.
    inner: aio::LioCb<'a>,
    sev: Mutex<SigevNotify>
}
// LCOV_EXCL_STOP

impl<'a> LioCb<'a> {
    /// Translate the operating system's somewhat unhelpful error from
    /// `lio_listio` into something more useful.
    fn fix_submit_error(&mut self, e: nix::Result<()>) -> Result<(), LioError> {
        match e {
            Err(nix::Error::EAGAIN) |
            Err(nix::Error::EIO) |
            Err(nix::Error::EINTR) => {
                // Unfortunately, FreeBSD uses EIO to indicate almost any
                // problem with lio_listio.  We must examine every aiocb to
                // determine which error to return
                let mut n_error = 0;
                let mut n_einprogress = 0;
                let mut n_eagain = 0;
                let mut n_ok = 0;
                let errors = (0..self.inner.len())
                .map(|i| {
                    self.inner.error(i)
                }).collect::<Vec<_>>();
                for (i, e) in errors.iter().enumerate() {
                    match e {
                        Ok(()) => {
                            n_ok += 1;
                        },
                        Err(Errno::EINPROGRESS) => {
                            n_einprogress += 1;
                        },
                        Err(Errno::EAGAIN) => {
                            n_eagain += 1;
                        },
                        Err(_) => {
                            // Depending on whether the operation was actually
                            // submitted or not, the kernel  may or may not
                            // require us to call aio_return. But Nix requires
                            // that we do, so it doesn't look like a resource
                            // leak.
                            let _ = self.inner.aio_return(i);
                            n_error += 1;
                        }
                    }
                }
                if n_error > 0 {
                    // Collect final status for every operation
                    Err(LioError::EIO(errors))
                } else if n_eagain > 0 && n_eagain < self.inner.len() {
                    Err(LioError::EINCOMPLETE)
                } else if n_eagain == self.inner.len() {
                    Err(LioError::EAGAIN)
                } else {
                    panic!("lio_listio returned EIO for unknown reasons.  n_error={}, n_einprogress={}, n_eagain={}, and n_ok={}",
                        n_error, n_einprogress, n_eagain, n_ok);
                }
            },
            Ok(()) => Ok(()),
            _ => panic!("lio_listio returned unhandled error {:?}", e)
        }
    }

    /// Submit an `LioCb` to the `aio(4)` subsystem.
    ///
    /// If the return value is [`LioError::EAGAIN`], then no operations were
    /// enqueued due to system resource limitations.  The application should
    /// free up resources and try again.  If the return value is
    /// [`LioError::EINCOMPLETE`], then _some_ operations were enqueued, but
    /// others were not, due to system resource limitations.  The application
    /// should wait for notification that the enqueued operations are complete,
    /// then resubmit the others with [`resubmit`](#method.resubmit).  If the
    /// return value is [`LioError::EIO`], then some operations have failed to
    /// enqueue, and cannot be resubmitted.  The application should wait for
    /// notification that the enqueued operations are complete, then examine the
    /// result of each operation to determine the problem.
    pub fn submit(&mut self) -> Result<(), LioError> {
        let sev =*self.sev.lock().unwrap();
        let e = self.inner.listio(aio::LioMode::LIO_NOWAIT, sev);
        self.fix_submit_error(e)
    }

    /// Resubmit an `LioCb` if it is incomplete.
    ///
    /// If [`submit`](#method.submit) returns `LioError::EINCOMPLETE`, then some
    /// operations may not have been submitted.  This method will collect status
    /// for any completed operations, then resubmit the others.
    ///
    /// [`lio_listio`](http://pubs.opengroup.org/onlinepubs/9699919799/functions/lio_listio.html)
    pub fn resubmit(&mut self) -> Result<(), LioError> {
        let sev =*self.sev.lock().unwrap();
        let e = self.inner.listio_resubmit(aio::LioMode::LIO_NOWAIT, sev);
        self.fix_submit_error(e)
    }

    /// Consume an `LioCb` and collect its operations' results.
    ///
    /// An iterator over all operations' results will be supplied to the
    /// callback function.
    // We can't simply return an iterator using self.inner.aiocbs.into_iter(),
    // because into_iter() moves elements, and aiocbs must reside at stable
    // memory locations.  This arrangement, though odd, avoids any large memory
    // allocations and still allows the caller to use an iterator adapter with
    // the results.
    pub fn into_results<F, R>(self, callback: F) -> R
        where F: FnOnce(Box<dyn Iterator<Item=LioResult> + 'a>) -> R {

        let mut inner = self.inner;
        let iter = (0..inner.len()).map(move |i| {
            let result = inner.aio_return(i);
            LioResult{result }
        });
        callback(Box::new(iter))
    }
}

impl<'a> Evented for LioCb<'a> {
    fn register(&self,
                poll: &Poll,
                token: Token,
                events: Ready,
                _: PollOpt) -> io::Result<()> {
        assert!(UnixReady::from(events).is_lio());
        let udata = usize::from(token);
        let kq = poll.as_raw_fd();
        let sigev = SigevNotify::SigevKevent{kq, udata: udata as isize};
        *self.sev.lock().unwrap() = sigev;
        Ok(())
    }

    fn reregister(&self,
                  poll: &Poll,
                  token: Token,
                  events: Ready,
                  opts: PollOpt) -> io::Result<()> {
        self.register(poll, token, events, opts)
    }

    fn deregister(&self, _: &Poll) -> io::Result<()> {
        let sigev = SigevNotify::SigevNone;
        *self.sev.lock().unwrap() = sigev;
        Ok(())
    }
}

/// Used to construct [`LioCb`].
///
/// `LioCb` uses the builder pattern. An `LioCbBuilder` is the only way to
/// construct an `LioCb`.
///
/// [`LioCb`](struct.LioCb.html)
#[derive(Debug)]
pub struct LioCbBuilder<'a>(aio::LioCbBuilder<'a>);

impl<'a> LioCbBuilder<'a> {
    pub fn emplace_mut_slice(self, fd: RawFd, offset: u64,
                         buf: &'a mut [u8], prio: i32, opcode: LioOpcode)
        -> Self
    {
        LioCbBuilder(
            self.0.emplace_mut_slice(
                fd,
                offset as off_t,
                buf,
                prio as c_int,
                SigevNotify::SigevNone,
                opcode
            )
        )
    }

    pub fn emplace_slice(self, fd: RawFd, offset: u64,
                         buf: &'a [u8], prio: i32, opcode: LioOpcode)
        -> Self
    {
        LioCbBuilder(
            self.0.emplace_slice(
                fd,
                offset as off_t,
                buf,
                prio as c_int,
                SigevNotify::SigevNone,
                opcode
            )
        )
    }

    pub fn finish(self) -> LioCb<'a> {
        LioCb {
            inner: self.0.finish(),
            sev: Mutex::new(SigevNotify::SigevNone)
        }
    }

    pub fn with_capacity(capacity: usize) -> LioCbBuilder<'a> {
        LioCbBuilder(aio::LioCbBuilder::with_capacity(capacity))
    }
}

/// Error types that can be returned by
/// [`LioCb::submit`](struct.LioCb.html#method.submit)
#[derive(Clone, Debug, PartialEq)]
pub enum LioError {
    /// No operations were enqueued.  No notification will be forthcoming.
    EAGAIN,
    /// Some operations were enqueued, but not all.  Notification will be
    /// delievered when the enqueued operations are all complete.
    EINCOMPLETE,
    /// Some operations failed.  The value is a vector of the status of each
    /// operation.
    EIO(Vec<Result<(), Errno>>)
}

impl LioError {
    pub fn into_eio(self) -> Result<Vec<Result<(), Errno>>, Self> {
        if let LioError::EIO(eio) = self {
            Ok(eio)
        } else {
            Err(self)
        }
    }
}

#[cfg(test)]
mod t {
    use super::*;
    use assert_impl::assert_impl;

    // It's important that `AioCb` and `LioCb` be `Sync` and `Send`.  Most Tokio
    // applications require it.
    #[test]
    fn aiocb_is_send_and_sync() {
        assert_impl!(Send: AioCb);
        assert_impl!(Sync: AioCb);
    }

    #[test]
    fn liocb_is_send_and_sync() {
        assert_impl!(Send: LioCb);
        assert_impl!(Sync: LioCb);
    }
}
