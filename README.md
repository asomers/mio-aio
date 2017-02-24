# mio-aio

A library for integration file I/O with [mio], using POSIX AIO and kqueue.  File I/O can be seamlessly mixed with network I/O and timers in the same event loop.

[mio]: https://github.com/carllerche/mio

```toml
# Cargo.toml
[dependencies]
mio-aio = "0.1"
mio = "0.6"
```

## Usage

TODO

# Platforms

`mio-aio` works on FreeBSD.  It will probably also work on DragonflyBSD and
OSX.  It does not work on Linux.

Unfortunately, Linux includes a poor implementation of POSIX AIO that emulates
asynchronous I/O in glibc using userland threads.  Worse, epoll(2) can't
deliver completion notifications for POSIX AIO.  That means that it can't be
supported by `mio-aio`.  But there's still hope for Linux users!  Linux has a
non-standard asynchronous file I/O API called libaio.  Libaio has better
performance than Linux's POSIX AIO.  It still can't deliver completion
notification throuh epoll(2), however.  What it can do is deliver completion
notification through a signal.  And using a signalfd(2), signal delivery
notification can be delivered through epoll(2).  So a Linux programmer wishing
to use `mio` with files could theoretically write a `mio-signalfd` crate and a
`mio-libaio` crate.  Then he could implement a portability layer above `mio`,
for example in `tokio`.

# License

`mio-aio` is primarily distributed under the terms of both the MIT license and
the Apache License (Version 2.0), with portions covered by various BSD-like
licenses.

See LICENSE-APACHE, and LICENSE-MIT for details.
