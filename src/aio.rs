use nix::libc::{c_int, off_t};
use mio::{Evented, Poll, Token, Ready, PollOpt};
use mio::unix::UnixReady;
use nix;
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
    BoxedSlice(Box<Borrow<[u8]>>),
    /// Mutable generic boxed slice
    BoxedMutSlice(Box<BorrowMut<[u8]>>)
}

impl BufRef {
    /// Return the inner `BoxedSlice`, if any
    pub fn boxed_slice(&self) -> Option<&Box<Borrow<[u8]>>> {
        match self {
            &BufRef::BoxedSlice(ref x) => Some(x),
            _ => None
        }
    }

    /// Return the inner `BoxedMutSlice`, if any
    pub fn boxed_mut_slice(&mut self) -> Option<&mut Box<BorrowMut<[u8]>>> {
        match self {
            &mut BufRef::BoxedMutSlice(ref mut x) => Some(x),
            _ => None
        }
    }

    /// Is this `BufRef` `None`?
    pub fn is_none(&self) -> bool {
        match self {
            &BufRef::None => true,
            _ => false,
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

#[derive(Debug)]
pub struct AioCb<'a> {
    // Must use a RefCell because mio::Evented's methods only take immutable
    // references, and unlike sockets, registering aiocb's requires modifying
    // the aiocb.  Must use Box for the AioCb so its location in memory will be
    // constant.  It is an error to move a libc::aiocb after passing it to the
    // kernel.
    inner: RefCell<Box<aio::AioCb<'a>>>,
}

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
    pub fn from_boxed_slice(fd: RawFd, offs: off_t, buf: Box<Borrow<[u8]>>,
                            prio: c_int, opcode: LioOpcode) -> AioCb<'a> {
        let aiocb = aio::AioCb::from_boxed_slice(fd, offs, buf, prio,
            SigevNotify::SigevNone, opcode);
        AioCb { inner: RefCell::new(Box::new(aiocb)) }
    }

    /// Creates a nix::sys::aio::AioCb from almost any kind of mutable boxed
    /// slice
    pub fn from_boxed_mut_slice(fd: RawFd, offs: off_t,
                                buf: Box<BorrowMut<[u8]>>, prio: c_int,
                                opcode: LioOpcode) -> AioCb<'a> {
        let aiocb = aio::AioCb::from_boxed_mut_slice(fd, offs, buf, prio,
            SigevNotify::SigevNone, opcode);
        AioCb { inner: RefCell::new(Box::new(aiocb)) }
    }

    /// Wraps nix::sys::aio::from_mut_slice
    ///
    /// Not as useful as it sounds, because in typical mio use cases, the
    /// compiler can't guarantee that the slice's lifetime is respected.
    pub fn from_mut_slice(fd: RawFd, offs: off_t, buf: &'a mut [u8],
                      prio: c_int, opcode: LioOpcode) -> AioCb {
        let aiocb = aio::AioCb::from_mut_slice(fd, offs, buf, prio,
                                           SigevNotify::SigevNone, opcode);
        AioCb { inner: RefCell::new(Box::new(aiocb)) }
    }

    /// Wraps nix::sys::aio::from_slice
    ///
    /// Mostly useful for writing constant slices
    pub fn from_slice(fd: RawFd, offs: off_t, buf: &'a [u8],
                      prio: c_int, opcode: LioOpcode) -> AioCb {
        let aiocb = aio::AioCb::from_slice(fd, offs, buf, prio,
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
    }

    /// Wrapper for nix::sys::aio::AioCb::cancel
    pub fn cancel(&self) -> nix::Result<aio::AioCancelStat> {
        self.inner.borrow_mut().cancel()
    }

    /// Wrapper for `nix::sys::aio::AioCb::error`
    pub fn error(&self) -> nix::Result<()> {
        self.inner.borrow_mut().error()
    }

    /// Wrapper for nix::sys::aio::AioCb::fsync
    pub fn fsync(&self, mode: AioFsyncMode) -> nix::Result<()> {
        self.inner.borrow_mut().fsync(mode)
    }

    /// Wrapper for nix::sys::aio::AioCb::read
    pub fn read(&self) -> nix::Result<()> {
        self.inner.borrow_mut().read()
    }

    /// Wrapper for nix::sys::aio::AioCb::write
    pub fn write(&self) -> nix::Result<()> {
        self.inner.borrow_mut().write()
    }
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
        let sigev = SigevNotify::SigevKevent{kq: kq, udata: udata as isize};
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


#[derive(Debug)]
pub struct LioCb {
    // Unlike AioCb, registering this structure does not modify the AioCb's
    // themselves, so no RefCell is needed.
    inner: aio::LioCb<'static>,
    // A plain Cell suffices, because we can Copy SigevNotify's.
    sev: Cell<SigevNotify>
}

impl LioCb {
    pub fn submit(&mut self) -> nix::Result<()> {
        self.inner.listio(aio::LioMode::LIO_NOWAIT, self.sev.get())
    }

    pub fn emplace_boxed_slice(&mut self, fd: RawFd, offset: off_t,
        buf: Box<Borrow<[u8]>>, prio: i32, opcode: LioOpcode) {
        self.inner.aiocbs.push(aio::AioCb::from_boxed_slice(fd, offset, buf,
            prio as c_int, SigevNotify::SigevNone, opcode))

    }

    pub fn emplace_boxed_mut_slice(&mut self, fd: RawFd, offset: off_t,
        buf: Box<BorrowMut<[u8]>>, prio: i32, opcode: LioOpcode) {
        self.inner.aiocbs.push(aio::AioCb::from_boxed_mut_slice(fd, offset, buf,
            prio as c_int, SigevNotify::SigevNone, opcode))
    }

    pub fn emplace_slice(&mut self, fd: RawFd, offset: off_t,
                         buf: &'static [u8], prio: i32, opcode: LioOpcode) {
        let aiocb = aio::AioCb::from_slice(fd, offset, buf, prio as c_int,
                                           SigevNotify::SigevNone, opcode);
        self.inner.aiocbs.push(aiocb);
    }

    /// Consume the `LioCb` and return its inners `AioCb`s
    ///
    /// It is an error to call this method while the `LioCb` is still in
    /// progress.
    pub fn into_results(mut self) -> Box<Iterator<Item = LioResult>> {
        // We can't simply use self.inner.aiocbs.iter_mut(), because iter_mut()
        // will move the elements, and aio_return relies on them having a stable
        // location.  So we must build a collection of the aio_return values,
        // then zip that with the BufRefs
        let results : Vec<nix::Result<isize>> = self.inner.aiocbs.iter_mut()
            .map(|ref mut aiocb| {
                aiocb.aio_return()
            }).collect();
        let consuming_iter = self.inner.aiocbs.into_iter().map(|mut aiocb| {
            nix_buffer_to_buf_ref(aiocb.buffer())
        });

        Box::new(results.into_iter().zip(consuming_iter).map(|(r, b)| {
            LioResult {
                result: r,
                buf_ref: b,
            }
        }))
    }

    pub fn with_capacity(capacity: usize) -> LioCb {
        LioCb {
            inner: aio::LioCb::with_capacity(capacity),
            sev: Cell::new(SigevNotify::SigevNone)
        }
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
        let sigev = SigevNotify::SigevKevent{kq: kq, udata: udata as isize};
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
