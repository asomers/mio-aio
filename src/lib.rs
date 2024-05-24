//! MIO bindings for POSIX AIO
//!
//! # Feature Flags
//!
//! * `tokio` - Add extra methods needed for consumers to implement Tokio's
//!             `AioSource` trait.
//!
//! # See Also
//!
//! * [`tokio-file`](https://docs.rs/tokio-file) - Tokio bindings that work atop
//!   this crate.  Much more useful to the typical user.
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod aio;

pub use aio::{AioFsyncMode, Source, Fsync, SourceApi, ReadAt, ReadvAt, WriteAt,
    WritevAt};
pub use nix::errno::Errno;
