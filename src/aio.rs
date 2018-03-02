use bytes::{Bytes, BytesMut};
use divbuf::{DivBuf, DivBufMut};
use nix::libc::{c_int, c_void, off_t};
use mio::{Evented, Poll, Token, Ready, PollOpt};
use mio::unix::UnixReady;
use nix;
use nix::sys::aio;
use nix::sys::signal::SigevNotify;
use std::cell::{Cell, RefCell};
use std::io;
use std::iter::{Iterator, FromIterator};
use std::mem;
use std::ops::{Deref, DerefMut, Index, IndexMut};
use std::os::unix::io::AsRawFd;
use std::os::unix::io::RawFd;
use std::slice;

pub use nix::sys::aio::AioFsyncMode;
pub use nix::sys::aio::LioOpcode;

/// Stores a reference to the buffer used by the `AioCb`, if any.
///
/// After the I/O operation is done, can be retrieved by `into_buf_ref`
#[derive(Debug)]
pub enum BufRef {
    /// Either the `AioCb` has no buffer, as for an fsync operation, or a
    /// reference can't be stored, as when constructed from a slice
    None,
    /// Immutable shared ownership `Bytes` object
    // Must use out-of-line allocation so the address of the data will be
    // stable.  Bytes and BytesMut sometimes dynamically allocate a buffer, and
    // sometimes inline the data within the struct itself.
    Bytes(Bytes),
    /// Mutable uniquely owned `BytesMut` object
    BytesMut(BytesMut),
    /// Immutable shared ownership `DivBuf` object
    DivBuf(DivBuf),
    /// Mutable shared ownership `DivBufMut` object
    DivBufMut(DivBufMut)
}

impl BufRef {
    /// Return the inner `Bytes`, if any
    pub fn bytes(&self) -> Option<&Bytes> {
        match self {
            &BufRef::Bytes(ref x) => Some(x),
            _ => None
        }
    }

    /// Return the inner `BytesMut`, if any
    pub fn bytes_mut(&self) -> Option<&BytesMut> {
        match self {
            &BufRef::BytesMut(ref x) => Some(x),
            _ => None
        }
    }

    /// Return the inner `DivBuf`, if any
    pub fn divbuf(&self) -> Option<&DivBuf> {
        match self {
            &BufRef::DivBuf(ref x) => Some(x),
            _ => None
        }
    }

    /// Return the inner `DivBufMut`, if any
    pub fn divbuf_mut(&self) -> Option<&DivBufMut> {
        match self {
            &BufRef::DivBufMut(ref x) => Some(x),
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

#[derive(Debug)]
pub struct AioCb<'a> {
    // Must use a RefCell because mio::Evented's methods only take immutable
    // references, and unlike sockets, registering aiocb's requires modifying
    // the aiocb.  Must use Box for the AioCb so its location in memory will be
    // constant.  It is an error to move a libc::aiocb after passing it to the
    // kernel.
    inner: RefCell<Box<aio::AioCb<'a>>>,
    // buf_ref is needed to keep references from dropping, but only for data
    // types that nix doesn't understand.  It can also be used for transfering
    // ownership of uniquely owned buffers between mio_aio and higher level code
    buf_ref: BufRef
}

/// Wrapper around nix::sys::aio::AioCb.
///
/// Implements mio::Evented.  After creation, use mio::Evented::register to
/// connect to the event loop
impl<'a> AioCb<'a> {
    /// Wraps nix::sys::aio::AioCb::from_fd.
    pub fn from_fd(fd: RawFd, prio: c_int) -> AioCb<'a> {
        let aiocb = aio::AioCb::from_fd(fd, prio, SigevNotify::SigevNone);
        AioCb { inner: RefCell::new(Box::new(aiocb)), buf_ref: BufRef::None }
    }

    /// Creates a nix::sys::aio::AioCb from a bytes::Bytes slice
    pub fn from_bytes(fd: RawFd, offs: off_t, buf: Bytes, prio: c_int,
                      opcode: LioOpcode) -> AioCb<'a> {
        // Small Bytes are stored inline.  Inline storage is a no-no, because we
        // store a pointer to the buffer in the AioCb before returning the
        // BufRef by move.  If the buffer is too small, reallocate it to force
        // out-of-line storage
        // TODO: Add an is_inline() method to Bytes, and a way to explicitly
        // force out-of-line allocation.
        let buf2 = if buf.len() < 64 {
            // Reallocate to force out-of-line allocation
            let mut ool = Bytes::with_capacity(64);
            ool.extend_from_slice(buf.deref());
            ool
        } else {
            buf
        };
        // Safety: ok because we keep a reference to buf2 in buf_ref
        let aiocb = unsafe {
            aio::AioCb::from_ptr(fd, offs,
                                 buf2.as_ptr() as *const c_void,
                                 buf2.len(), prio, SigevNotify::SigevNone,
                                 opcode)
        };
        let buf_ref = BufRef::Bytes(buf2);
        AioCb { inner: RefCell::new(Box::new(aiocb)), buf_ref: buf_ref}
    }

    /// Creates a nix::sys::aio::AioCb from a bytes::BytesMut slice
    pub fn from_bytes_mut(fd: RawFd, offs: off_t, buf: BytesMut,
                          prio: c_int, opcode: LioOpcode) -> AioCb<'a> {
        // Small BytesMuts are stored inline.  Inline storage is a no-no,
        // because we store a pointer to the buffer in the AioCb before
        // returning the BufRef by move.  If the buffer is too small, reallocate
        // it to force out-of-line storage
        // TODO: Add an is_inline() method to BytesMut, and a way to explicitly
        // force out-of-line allocation.
        let mut buf2 = if buf.len() < 64 {
            let mut ool = BytesMut::with_capacity(64);
            ool.extend_from_slice(buf.deref());
            ool
        } else {
            buf
        };
        // Safety: ok because we keep a reference to buf2 in buf_ref
        let aiocb = unsafe {
            aio::AioCb::from_mut_ptr(fd, offs,
                                     buf2.as_mut_ptr() as *mut c_void,
                                     buf2.len(), prio, SigevNotify::SigevNone,
                                     opcode)
        };
        let buf_ref = BufRef::BytesMut(buf2);
        AioCb { inner: RefCell::new(Box::new(aiocb)), buf_ref: buf_ref}
    }

    /// Creates a nix::sys::aio::AioCb from a divbuf::DivBuf slice
    pub fn from_divbuf(fd: RawFd, offs: off_t, buf: DivBuf, prio: c_int,
                      opcode: LioOpcode) -> AioCb<'a> {
        // Safety: ok because we keep a reference to buf in buf_ref
        let aiocb = unsafe {
            aio::AioCb::from_ptr(fd, offs,
                                 buf.as_ptr() as *const c_void,
                                 buf.len(), prio, SigevNotify::SigevNone,
                                 opcode)
        };
        let buf_ref = BufRef::DivBuf(buf);
        AioCb { inner: RefCell::new(Box::new(aiocb)), buf_ref: buf_ref}
    }

    /// Creates a nix::sys::aio::AioCb from a divbuf::DivBufMut slice
    pub fn from_divbuf_mut(fd: RawFd, offs: off_t, mut buf: DivBufMut,
                          prio: c_int, opcode: LioOpcode) -> AioCb<'a> {
        // Safety: ok because we keep a reference to buf in buf_ref
        let aiocb = unsafe {
            aio::AioCb::from_mut_ptr(fd, offs,
                                     buf.as_mut_ptr() as *mut c_void,
                                     buf.len(), prio, SigevNotify::SigevNone,
                                     opcode)
        };
        let buf_ref = BufRef::DivBufMut(buf);
        AioCb { inner: RefCell::new(Box::new(aiocb)), buf_ref: buf_ref}
    }

    /// Wraps nix::sys::aio::from_mut_slice
    ///
    /// Not as useful as it sounds, because in typical mio use cases, the
    /// compiler can't guarantee that the slice's lifetime is respected.
    pub fn from_mut_slice(fd: RawFd, offs: off_t, buf: &'a mut [u8],
                      prio: c_int, opcode: LioOpcode) -> AioCb {
        let aiocb = aio::AioCb::from_mut_slice(fd, offs, buf, prio,
                                           SigevNotify::SigevNone, opcode);
        AioCb { inner: RefCell::new(Box::new(aiocb)), buf_ref: BufRef::None }
    }

    /// Wraps nix::sys::aio::from_slice
    ///
    /// Mostly useful for writing constant slices
    pub fn from_slice(fd: RawFd, offs: off_t, buf: &'a [u8],
                      prio: c_int, opcode: LioOpcode) -> AioCb {
        let aiocb = aio::AioCb::from_slice(fd, offs, buf, prio,
                                           SigevNotify::SigevNone, opcode);
        AioCb { inner: RefCell::new(Box::new(aiocb)), buf_ref: BufRef::None  }
    }

    /// Remove the `AioCb`'s inner `BufRef` and return it
    ///
    /// It is an error to call this method while the `AioCb` is still in
    /// progress.
    ///
    /// XXX This method is only temporary; it will be removed once tokio_core
    /// supports a `PollEvented::into_inner` method.
    ///
    /// # Safety
    ///
    /// This method can cause an `AioCb`'s inner `BufRef` to be `drop`ped while
    /// the kernel still has a reference, which makes it unsafe.
    pub unsafe fn buf_ref(&mut self) -> BufRef {
        let mut x = BufRef::None;
        mem::swap(&mut self.buf_ref, &mut x);
        x
    }

    /// Consume the `AioCb` and return its inner `BufRef`
    ///
    /// It is an error to call this method while the `AioCb` is still in
    /// progress.
    pub fn into_buf_ref(self) -> BufRef {
        self.buf_ref
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
pub struct LioCb<'a> {
    // Unlike AioCb, registering this structure does not modify the AioCb's
    // themselves, so no RefCell is needed.  But we need buf_ref, so it's easier
    // to simply reuse the mio_aio::AioCb struct, even though we don't need the
    // RefCell
    inner: Vec<AioCb<'a>>,
    // A plain Cell suffices, because we can Copy SigevNotify's.
    sev: Cell<SigevNotify>
}

impl<'a> LioCb<'a> {
    pub fn listio(&mut self) -> nix::Result<()> {
        // Pacify the borrow checker.  We need to keep these RefMut objects
        // around until after aiolist goes out of scope
        let mut vec_of_refmuts = Vec::from_iter(
            self.inner.iter().map(|mio_aiocb| {
                mio_aiocb.inner.borrow_mut()
            }));
        // Pacify the borrow checker.  We need to keep vec_of_refs around until
        // aiolist goes out of scope.  If we chain the call to as_slice, the
        // vector drops too soon.
        let vec_of_refs = Vec::from_iter(vec_of_refmuts.iter_mut().map(|rm| {
            rm.deref_mut().deref_mut()
        }));
        let aiolist = vec_of_refs.as_slice();
        aio::lio_listio(aio::LioMode::LIO_NOWAIT, aiolist, self.sev.get())
    }

    pub fn emplace_bytes(&mut self, fd: RawFd, offset: off_t,
                         buf: Bytes, prio: i32, opcode: LioOpcode) {
        let aiocb = AioCb::from_bytes(fd, offset, buf, prio as c_int, opcode);
        self.inner.push(aiocb);
    }

    pub fn emplace_bytes_mut(&mut self, fd: RawFd, offset: off_t,
                         buf: BytesMut, prio: i32, opcode: LioOpcode) {
        let aiocb = AioCb::from_bytes_mut(fd, offset, buf, prio as c_int,
                                          opcode);
        self.inner.push(aiocb);
    }

    pub fn emplace_divbuf(&mut self, fd: RawFd, offset: off_t,
                         buf: DivBuf, prio: i32, opcode: LioOpcode) {
        let aiocb = AioCb::from_divbuf(fd, offset, buf, prio as c_int, opcode);
        self.inner.push(aiocb);
    }

    pub fn emplace_divbuf_mut(&mut self, fd: RawFd, offset: off_t,
                         buf: DivBufMut, prio: i32, opcode: LioOpcode) {
        let aiocb = AioCb::from_divbuf_mut(fd, offset, buf, prio as c_int,
                                          opcode);
        self.inner.push(aiocb);
    }

    pub fn emplace_slice(&mut self, fd: RawFd, offset: off_t,
                         buf: &'a [u8], prio: i32, opcode: LioOpcode) {
        let aiocb = AioCb::from_slice(fd, offset, buf, prio as c_int, opcode);
        self.inner.push(aiocb);
    }

    /// Consume the `LioCb` and return its inners `AioCb`s
    ///
    /// It is an error to call this method while the `LioCb` is still in
    /// progress.
    pub fn into_aiocbs(self) -> Box<Iterator<Item = AioCb<'a>> + 'a> {
        Box::new(self.inner.into_iter())
    }

    /// Iterate over all `AioCb` contained within the `LioCb`
    pub fn iter(&self) -> slice::Iter<AioCb<'a>> {
        self.inner.iter()
    }

    /// Mutably iterate over all `AioCb` contained within the `LioCb`
    pub fn iter_mut(&mut self) -> slice::IterMut<AioCb<'a>> {
        self.inner.iter_mut()
    }

    pub fn with_capacity(capacity: usize) -> LioCb<'a> {
        LioCb {
            inner: Vec::<AioCb<'a>>::with_capacity(capacity),
            sev: Cell::new(SigevNotify::SigevNone)
        }
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

impl<'a> IntoIterator for LioCb<'a> {
    type Item = AioCb<'a>;
    type IntoIter = ::std::vec::IntoIter<AioCb<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<'a> Index<usize> for LioCb<'a> {
    type Output = AioCb<'a>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.inner[index]
    }
}

impl<'a> IndexMut<usize> for LioCb<'a> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.inner[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_lio_iter() {
        let f : RawFd = 10042;
        const WBUF: &'static [u8] = b"abcdef";

        let mut liocb = LioCb::with_capacity(2);
        liocb.emplace_slice(f, 2, WBUF, 0, LioOpcode::LIO_WRITE);
        liocb.emplace_slice(f, 7, WBUF, 0, LioOpcode::LIO_WRITE);
        let mut iter = liocb.iter();
        let first = iter.next().unwrap();
        assert_eq!(2, first.inner.borrow().offset());
        let second = iter.next().unwrap();
        assert_eq!(7, second.inner.borrow().offset());
    }

    // Only a few `AioCb` methods need a mutable reference.  `error` is one
    #[test]
    pub fn test_lio_iter_mut() {
        let f : RawFd = 10042;
        const WBUF: &'static [u8] = b"abcdef";

        let mut liocb = LioCb::with_capacity(2);
        liocb.emplace_slice(f, 2, WBUF, 0, LioOpcode::LIO_WRITE);
        liocb.emplace_slice(f, 7, WBUF, 0, LioOpcode::LIO_WRITE);
        let mut iter = liocb.iter_mut();
        let first = iter.next().unwrap();
        assert!(first.error().is_err());
        let second = iter.next().unwrap();
        assert!(second.error().is_err());
    }

    #[test]
    pub fn test_lio_into_iter() {
        let f : RawFd = 10042;
        const WBUF: &'static [u8] = b"abcdef";

        let mut liocb = LioCb::with_capacity(2);
        liocb.emplace_slice(f, 2, WBUF, 0, LioOpcode::LIO_WRITE);
        liocb.emplace_slice(f, 7, WBUF, 0, LioOpcode::LIO_WRITE);
        let mut iter = liocb.into_iter();
        let first = iter.next().unwrap();
        assert_eq!(2, first.inner.borrow().offset());
        let second = iter.next().unwrap();
        assert_eq!(7, second.inner.borrow().offset());
    }
}
