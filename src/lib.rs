//! MIO bindings for POSIX AIO

mod aio;

pub use aio::{AioCb, AioFsyncMode, LioCb, LioCbBuilder, LioOpcode,
    LioError};
pub use nix::errno::Errno;
