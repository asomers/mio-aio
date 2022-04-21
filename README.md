# mio-aio

A library for integrating file I/O with [mio], using POSIX AIO and kqueue.
File I/O can be seamlessly mixed with network I/O and timers in the same event
loop.

[![Build Status](https://api.cirrus-ci.com/github/asomers/mio-aio.svg)](https://cirrus-ci.com/github/asomers/mio-aio)
[mio]: https://github.com/carllerche/mio

```toml
# Cargo.toml
[dependencies]
mio-aio = "0.7.0"
mio = "0.8.0"
```

## Usage

Usage of this crate is based on the `mio_aio::AioCb` type, which is a wrapper
around `nix::AioCb`.  You can create one using constructors that are similar to
what `nix` provides.  Registration is the same as any `mio` type, except that
`AioCb` must be individually registered.  The underlying file does not get
registered with `mio`.  Once registered, you can issue the `AioCb` using
methods that wrap the `nix` type: `read`, `write`, etc.  After `mio`'s `poll`
method returns the event, call `AioCb::aio_return` to get the final status.  At
this point, the kernel has forgotten about the `AioCb`.  There is no need to
deregister it (though deregistration does not hurt).


# Platforms

`mio-aio` works on FreeBSD.  It will probably also work on DragonflyBSD.
It does not work on Linux or MacOS.

Unfortunately, Linux includes a poor implementation of POSIX AIO that emulates
asynchronous I/O in glibc using userland threads.  Worse, epoll(2) can't
deliver completion notifications for POSIX AIO.  That means that it can't be
supported by `mio-aio`.  But there's still hope for Linux users!  Linux has a
non-standard asynchronous file I/O API called libaio.  Libaio has better
performance than Linux's POSIX AIO.  It still can't deliver completion
notification throuh epoll(2), however.  What it can do is deliver completion
notification through an eventfd(2).  And epoll can poll an eventfd.  So a Linux
programmer wishing to use `mio` with files could theoretically write a
`mio-libaio` crate that uses one eventfd per reactor to poll all libaio
operations .  Then he could implement a portability layer above `mio`, for
example in `tokio`.

On MacOS AIO only supports notification using signals, not kqueue.  On MacOS
`mio-aio` could theoretically run `aio-suspend` in a separate thread, which
would send completion notification to the main thread's reactor.  Performance
would suffer, however.

# License

`mio-aio` is primarily distributed under the terms of both the MIT license and
the Apache License (Version 2.0).

See LICENSE-APACHE, and LICENSE-MIT for details.
