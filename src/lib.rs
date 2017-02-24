//! MIO bindings for POSIX AIO

extern crate libc;
extern crate mio;
extern crate nix;

mod aio;

pub use aio::AioCb;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}
