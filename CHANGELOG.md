## [Unreleased] - ReleaseDate

### Changed

- Updated Nix to 0.22.0.  This changes mio-aio's error types, because we
  reexport from Nix.
  (#[21](https://github.com/asomers/mio-aio/pull/21))

- Updated mio to 0.7.

## [0.5.0] - 2021-05-31

### Changed

- mio-aio's operations no longer own their buffers.  It is less necessary now
  that async/await is available.  Instead, all mio-aio operations use borrowed
  buffers.

- Most `AioCb` methods now take a mutable receiver rather than an immutable one.

## [0.4.1] - 2019-08-07
### Fixed
- Fixed several dependencies's version specifications.

## [0.4.0] - 2018-11-29
### Added
- Added `BufRef::len`

### Changed
- If an `lio_listio` operation fails asynchronously, the future will now
  include final error status for all failed operations.
- `BufRef::boxed_slice` and `BufRef::boxed_mut_slice` now return `&Borrow` and
  `&BorrowMut` respectively, rather than references to the boxed type.

## [0.3.1] - 2018-07-01
### Fixed
- Fixed Cargo's documentation link

## [0.3.0] - 2018-07-01
### Added
- Support for submitting multiple operations at once with `lio_listio`.

### Changed
- `AioCb` structures can no longer be created from a `Rc<Box<[u8]>>`.  Use a
  `Box<Borrow<[u8]>>` or a `Box<BorrowMut<[u8]>>` instead.

### Fixed

### Removed
