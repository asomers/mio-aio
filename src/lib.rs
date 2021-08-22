//! MIO bindings for POSIX AIO
//!
//! # Feature Flags
//!
//! * `tokio` - Add extra methods needed for consumers to implement Tokio's
//!             `AioSource` trait.
#![cfg_attr(docsrs, feature(doc_cfg))]

mod aio;

pub use aio::{AioCb, AioFsyncMode, LioCb, LioCbBuilder, LioOpcode,
    LioError};
pub use nix::errno::Errno;
