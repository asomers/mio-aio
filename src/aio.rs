// vim: tw=80
use nix::libc::{c_int, off_t};
use mio::{Evented, Poll, Token, Ready, PollOpt};
use mio::unix::UnixReady;
use nix;
use nix::errno::Errno;
use nix::sys::aio;
use nix::sys::signal::SigevNotify;
use std::borrow::{Borrow, BorrowMut};
use std::cell::{Cell, RefCell};
use std::io;
use std::iter::Iterator;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::RawFd;

pub use nix::sys::aio::AioFsyncMode;
pub use nix::sys::aio::LioOpcode;


/// Stores a reference to the buffer used by the `AioCb`, if any.
///
/// After the I/O operation is done, can be retrieved by `buf_ref`
pub enum BufRef {
    /// Either the `AioCb` has no buffer, as for an fsync operation, or a
    /// reference can't be stored, as when constructed from a slice
    None,
    /// Immutable generic boxed slice
    BoxedSlice(Box<dyn Borrow<[u8]>>),
    /// Mutable generic boxed slice
    BoxedMutSlice(Box<dyn BorrowMut<[u8]>>)
}

// is_empty wouldn't make sense because our len returns an Option
#[cfg_attr(feature = "cargo-clippy", allow(clippy::len_without_is_empty))]
impl BufRef {
    /// Return the inner `BoxedSlice`, if any
    pub fn boxed_slice(&self) -> Option<&dyn Borrow<[u8]>> {
        match *self {
            BufRef::BoxedSlice(ref x) => Some(x.as_ref()),
            _ => None
        }
    }

    /// Return the inner `BoxedMutSlice`, if any
    pub fn boxed_mut_slice(&mut self) -> Option<&mut dyn BorrowMut<[u8]>> {
        match *self {
            BufRef::BoxedMutSlice(ref mut x) => Some(x.as_mut()),
            _ => None
        }
    }

    /// Is this `BufRef` `None`?
    pub fn is_none(&self) -> bool {
        match *self {
            BufRef::None => true,
            _ => false,
        }
    }

    /// Length of the buffer, if any
    pub fn len(&self) -> Option<usize> {
        match *self {
            BufRef::BoxedSlice(ref x) => Some(x.as_ref().borrow().len()),
            BufRef::BoxedMutSlice(ref x) => Some(x.as_ref().borrow().len()),
            BufRef::None => None
        }
    }
}


/// Consume a nix::sys::aio::Buffer and return a mio_aio::BufRef
fn nix_buffer_to_buf_ref(b: aio::Buffer) -> BufRef {
    match b {
        aio::Buffer::BoxedSlice(x) => BufRef::BoxedSlice(x),
        aio::Buffer::BoxedMutSlice(x) => BufRef::BoxedMutSlice(x),
        _ => BufRef::None
    }
}

/// Represents the result of an individual operation from an `LioCb::submit`
/// call.
pub struct LioResult {
    pub buf_ref: BufRef,
    pub result: nix::Result<isize>
}

// LCOV_EXCL_START
#[derive(Debug)]
pub struct AioCb<'a> {
    // Must use a RefCell because mio::Evented's methods only take immutable
    // references, and unlike sockets, registering aiocb's requires modifying
    // the aiocb.  Must use Box for the AioCb so its location in memory will be
    // constant.  It is an error to move a libc::aiocb after passing it to the
    // kernel.
    inner: RefCell<Box<aio::AioCb<'a>>>,
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
        AioCb { inner: RefCell::new(Box::new(aiocb)) }
    }

    /// Creates a nix::sys::aio::AioCb from almost any kind of boxed slice
    pub fn from_boxed_slice(fd: RawFd, offs: u64, buf: Box<dyn Borrow<[u8]>>,
                            prio: c_int, opcode: LioOpcode) -> AioCb<'a> {
        let aiocb = aio::AioCb::from_boxed_slice(fd, offs as off_t, buf, prio,
            SigevNotify::SigevNone, opcode);
        AioCb { inner: RefCell::new(Box::new(aiocb)) }
    }

    /// Creates a nix::sys::aio::AioCb from almost any kind of mutable boxed
    /// slice
    pub fn from_boxed_mut_slice(fd: RawFd, offs: u64,
                                buf: Box<dyn BorrowMut<[u8]>>, prio: c_int,
                                opcode: LioOpcode) -> AioCb<'a> {
        let aiocb = aio::AioCb::from_boxed_mut_slice(fd, offs as off_t, buf,
            prio, SigevNotify::SigevNone, opcode);
        AioCb { inner: RefCell::new(Box::new(aiocb)) }
    }

    /// Wraps nix::sys::aio::from_mut_slice
    ///
    /// Not as useful as it sounds, because in typical mio use cases, the
    /// compiler can't guarantee that the slice's lifetime is respected.
    pub fn from_mut_slice(fd: RawFd, offs: u64, buf: &'a mut [u8],
                      prio: c_int, opcode: LioOpcode) -> AioCb {
        let aiocb = aio::AioCb::from_mut_slice(fd, offs as off_t, buf, prio,
                                           SigevNotify::SigevNone, opcode);
        AioCb { inner: RefCell::new(Box::new(aiocb)) }
    }

    /// Wraps nix::sys::aio::from_slice
    ///
    /// Mostly useful for writing constant slices
    pub fn from_slice(fd: RawFd, offs: u64, buf: &'a [u8],
                      prio: c_int, opcode: LioOpcode) -> AioCb {
        let aiocb = aio::AioCb::from_slice(fd, offs as off_t, buf, prio,
                                           SigevNotify::SigevNone, opcode);
        AioCb { inner: RefCell::new(Box::new(aiocb)) }
    }

    /// return an `AioCb`'s inner `BufRef`
    ///
    /// It is an error to call this method while the `AioCb` is still in
    /// progress.
    pub fn buf_ref(&mut self) -> BufRef {
        nix_buffer_to_buf_ref(self.inner.borrow_mut().buffer())
    }

    /// Wrapper for nix::sys::aio::aio_return
    pub fn aio_return(&self) -> nix::Result<isize> {
        self.inner.borrow_mut().aio_return()
    }   // LCOV_EXCL_LINE

    /// Wrapper for nix::sys::aio::AioCb::cancel
    pub fn cancel(&self) -> nix::Result<aio::AioCancelStat> {
        self.inner.borrow_mut().cancel()
    }   // LCOV_EXCL_LINE

    /// Wrapper for `nix::sys::aio::AioCb::error`
    ///
    /// Not usually needed, since `mio_aio` always uses kqueue for notification.
    pub fn error(&self) -> nix::Result<()> {
        self.inner.borrow_mut().error()
    }   // LCOV_EXCL_LINE

    /// Wrapper for nix::sys::aio::AioCb::fsync
    pub fn fsync(&self, mode: AioFsyncMode) -> nix::Result<()> {
        self.inner.borrow_mut().fsync(mode)
    }   // LCOV_EXCL_LINE

    /// Wrapper for nix::sys::aio::AioCb::read
    pub fn read(&self) -> nix::Result<()> {
        self.inner.borrow_mut().read()
    }   // LCOV_EXCL_LINE

    /// Wrapper for nix::sys::aio::AioCb::write
    pub fn write(&self) -> nix::Result<()> {
        self.inner.borrow_mut().write()
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
        self.inner.borrow_mut().set_sigev_notify(sigev);
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
        self.inner.borrow_mut().set_sigev_notify(sigev);
        Ok(())
    }
}


// LCOV_EXCL_START
#[derive(Debug)]
pub struct LioCb {
    // Unlike AioCb, registering this structure does not modify the AioCb's
    // themselves, so no RefCell is needed.
    inner: aio::LioCb<'static>,
    // A plain Cell suffices, because we can Copy SigevNotify's.
    sev: Cell<SigevNotify>
}
// LCOV_EXCL_STOP

impl LioCb {
    /// Translate the operating system's somewhat unhelpful error from
    /// `lio_listio` into something more useful.
    fn fix_submit_error(&mut self, e: nix::Result<()>) -> Result<(), LioError> {
        match e {
            Err(nix::Error::Sys(nix::errno::Errno::EAGAIN)) |
            Err(nix::Error::Sys(nix::errno::Errno::EIO)) |
            Err(nix::Error::Sys(nix::errno::Errno::EINTR)) => {
                // Unfortunately, FreeBSD uses EIO to indicate almost any
                // problem with lio_listio.  We must examine every aiocb to
                // determine which error to return
                let mut n_error = 0;
                let mut n_einprogress = 0;
                let mut n_eagain = 0;
                let mut n_ok = 0;
                let errors = (0..self.inner.aiocbs.len())
                .map(|i| {
                    self.inner.error(i).map_err(|e| e.as_errno().unwrap())
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
                } else if n_eagain > 0 && n_eagain < self.inner.aiocbs.len() {
                    Err(LioError::EINCOMPLETE)
                } else if n_eagain == self.inner.aiocbs.len() {
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
        let e = self.inner.listio(aio::LioMode::LIO_NOWAIT, self.sev.get());
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
        let e = self.inner.listio_resubmit(aio::LioMode::LIO_NOWAIT, self.sev.get());
        self.fix_submit_error(e)
    }

    pub fn emplace_boxed_slice(&mut self, fd: RawFd, offset: u64,
        buf: Box<dyn Borrow<[u8]>>, prio: i32, opcode: LioOpcode) {
        self.inner.aiocbs.push(aio::AioCb::from_boxed_slice(fd, offset as off_t,
            buf, prio as c_int, SigevNotify::SigevNone, opcode))

    }

    pub fn emplace_boxed_mut_slice(&mut self, fd: RawFd, offset: u64,
        buf: Box<dyn BorrowMut<[u8]>>, prio: i32, opcode: LioOpcode) {
        self.inner.aiocbs.push(aio::AioCb::from_boxed_mut_slice(fd,
            offset as off_t, buf, prio as c_int, SigevNotify::SigevNone,
            opcode))
    }

    pub fn emplace_slice(&mut self, fd: RawFd, offset: u64,
                         buf: &'static [u8], prio: i32, opcode: LioOpcode) {
        let aiocb = aio::AioCb::from_slice(fd, offset as off_t, buf,
            prio as c_int, SigevNotify::SigevNone, opcode);
        self.inner.aiocbs.push(aiocb);
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
        where F: FnOnce(Box<dyn Iterator<Item=LioResult>>) -> R {

        let mut inner = self.inner;
        let iter = (0..inner.aiocbs.len()).map(move |i| {
            let result = inner.aio_return(i);
            let buf_ref = nix_buffer_to_buf_ref(inner.aiocbs[i].buffer());
            LioResult{result, buf_ref, }
        });
        callback(Box::new(iter))
    }

    pub fn with_capacity(capacity: usize) -> LioCb {
        LioCb {
            inner: aio::LioCb::with_capacity(capacity),
            sev: Cell::new(SigevNotify::SigevNone)
        }   // LCOV_EXCL_LINE
    }
}

impl Evented for LioCb {
    fn register(&self,
                poll: &Poll,
                token: Token,
                events: Ready,
                _: PollOpt) -> io::Result<()> {
        assert!(UnixReady::from(events).is_lio());
        let udata = usize::from(token);
        let kq = poll.as_raw_fd();
        let sigev = SigevNotify::SigevKevent{kq, udata: udata as isize};
        self.sev.set(sigev);
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
        self.sev.set(sigev);
        Ok(())
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
