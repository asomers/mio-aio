// vim: tw=80
use std::{
    io::{self, IoSlice, IoSliceMut},
    os::unix::io::{AsRawFd, BorrowedFd, RawFd},
    pin::Pin,
};

use mio::{event, Interest, Registry, Token};
pub use nix::sys::aio::AioFsyncMode;
use nix::{
    libc::off_t,
    sys::{
        aio::{self, Aio},
        event::EventFlag,
        signal::SigevNotify,
    },
};

/// Return type of [`Source::read_at`]
pub type ReadAt<'a> = Source<aio::AioRead<'a>>;
/// Return type of [`Source::readv_at`]
pub type ReadvAt<'a> = Source<aio::AioReadv<'a>>;
/// Return type of [`Source::fsync`]
pub type Fsync<'a> = Source<aio::AioFsync<'a>>;
/// Return type of [`Source::write_at`]
pub type WriteAt<'a> = Source<aio::AioWrite<'a>>;
/// Return type of [`Source::writev_at`]
pub type WritevAt<'a> = Source<aio::AioWritev<'a>>;

/// Common methods supported by all POSIX AIO Mio sources
pub trait SourceApi {
    /// Return type of [`SourceApi::aio_return`].
    type Output;

    /// Read the final result of the operation
    fn aio_return(self: Pin<&mut Self>) -> nix::Result<Self::Output>;

    /// Ask the operating system to cancel the operation
    ///
    /// Most file systems on most operating systems don't actually support
    /// cancellation; they'll just return `AIO_NOTCANCELED`.
    fn cancel(self: Pin<&mut Self>) -> nix::Result<aio::AioCancelStat>;

    /// Retrieve the status of an in-progress or complete operation.
    ///
    /// Not usually needed, since `mio_aio` always uses kqueue for notification.
    fn error(self: Pin<&mut Self>) -> nix::Result<()>;

    /// Does this operation currently have any in-kernel state?
    fn in_progress(&self) -> bool;

    /// Extra registration method needed by Tokio
    #[cfg(feature = "tokio")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
    fn deregister_raw(&mut self);

    /// Extra registration method needed by Tokio
    #[cfg(feature = "tokio")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
    fn register_raw(&mut self, kq: RawFd, udata: usize);

    /// Actually start the I/O operation.
    ///
    /// After calling this method and until [`SourceApi::aio_return`] returns
    /// `Ok`, the structure may not be moved in memory.
    fn submit(self: Pin<&mut Self>) -> nix::Result<()>;
}

/// A Mio source based on a single POSIX AIO operation.
///
/// The generic parameter specifies exactly which operation it is.  This struct
/// implements `mio::Source`.  After creation, use `mio::Source::register` to
/// connect it to the event loop.
#[derive(Debug)]
pub struct Source<T> {
    inner: T,
}
impl<T: Aio> Source<T> {
    pin_utils::unsafe_pinned!(inner: T);

    fn _deregister_raw(&mut self) {
        let sigev = SigevNotify::SigevNone;
        self.inner.set_sigev_notify(sigev);
    }

    fn _register_raw(&mut self, kq: RawFd, udata: usize) {
        let sigev = SigevNotify::SigevKeventFlags {
            kq,
            udata: udata as isize,
            flags: EventFlag::EV_ONESHOT,
        };
        self.inner.set_sigev_notify(sigev);
    }
}

impl<T: Aio> SourceApi for Source<T> {
    type Output = T::Output;

    fn aio_return(self: Pin<&mut Self>) -> nix::Result<Self::Output> {
        self.inner().aio_return()
    }

    fn cancel(self: Pin<&mut Self>) -> nix::Result<aio::AioCancelStat> {
        self.inner().cancel()
    }

    #[cfg(feature = "tokio")]
    fn deregister_raw(&mut self) {
        self._deregister_raw()
    }

    fn error(self: Pin<&mut Self>) -> nix::Result<()> {
        self.inner().error()
    }

    fn in_progress(&self) -> bool {
        self.inner.in_progress()
    }

    #[cfg(feature = "tokio")]
    fn register_raw(&mut self, kq: RawFd, udata: usize) {
        self._register_raw(kq, udata)
    }

    fn submit(self: Pin<&mut Self>) -> nix::Result<()> {
        self.inner().submit()
    }
}

impl<T: Aio> event::Source for Source<T> {
    fn register(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        assert!(interests.is_aio());
        let udata = usize::from(token);
        let kq = registry.as_raw_fd();
        self._register_raw(kq, udata);
        Ok(())
    }

    fn reregister(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        self.register(registry, token, interests)
    }

    fn deregister(&mut self, _registry: &Registry) -> io::Result<()> {
        self._deregister_raw();
        Ok(())
    }
}

impl<'a> Source<aio::AioFsync<'a>> {
    /// Asynchronously fsync a file.
    pub fn fsync(fd: BorrowedFd<'a>, mode: AioFsyncMode, prio: i32) -> Self {
        let inner = aio::AioFsync::new(fd, mode, prio, SigevNotify::SigevNone);
        Source { inner }
    }
}

impl<'a> Source<aio::AioRead<'a>> {
    /// Asynchronously read from a file.
    pub fn read_at(
        fd: BorrowedFd<'a>,
        offs: u64,
        buf: &'a mut [u8],
        prio: i32,
    ) -> Self {
        let inner = aio::AioRead::new(
            fd,
            offs as off_t,
            buf,
            prio,
            SigevNotify::SigevNone,
        );
        Source { inner }
    }
}

impl<'a> Source<aio::AioReadv<'a>> {
    /// Asynchronously read from a file to a scatter/gather list of buffers.
    ///
    /// Requires FreeBSD 13.0 or later.
    pub fn readv_at(
        fd: BorrowedFd<'a>,
        offs: u64,
        bufs: &mut [IoSliceMut<'a>],
        prio: i32,
    ) -> Self {
        let inner = aio::AioReadv::new(
            fd,
            offs as off_t,
            bufs,
            prio,
            SigevNotify::SigevNone,
        );
        Source { inner }
    }
}

impl<'a> Source<aio::AioWrite<'a>> {
    /// Asynchronously write to a file.
    pub fn write_at(
        fd: BorrowedFd<'a>,
        offs: u64,
        buf: &'a [u8],
        prio: i32,
    ) -> Self {
        let inner = aio::AioWrite::new(
            fd,
            offs as off_t,
            buf,
            prio,
            SigevNotify::SigevNone,
        );
        Source { inner }
    }
}

impl<'a> Source<aio::AioWritev<'a>> {
    /// Asynchronously write to a file to a scatter/gather list of buffers.
    ///
    /// Requires FreeBSD 13.0 or later.
    pub fn writev_at(
        fd: BorrowedFd<'a>,
        offs: u64,
        bufs: &[IoSlice<'a>],
        prio: i32,
    ) -> Self {
        let inner = aio::AioWritev::new(
            fd,
            offs as off_t,
            bufs,
            prio,
            SigevNotify::SigevNone,
        );
        Source { inner }
    }
}
