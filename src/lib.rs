//! MIO bindings for POSIX AIO

extern crate bytes;
extern crate mio;
extern crate nix;

mod aio;

pub use aio::{AioCb, AioFsyncMode, BufRef, LioCb, LioOpcode};
