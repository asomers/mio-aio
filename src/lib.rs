//! MIO bindings for POSIX AIO

extern crate mio;
extern crate nix;

mod aio;

pub use aio::{AioCb, AioFsyncMode, BufRef, LioCb, LioOpcode, LioError};
