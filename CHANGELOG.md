## [Unreleased] - ReleaseDate
### Changed
-  If an `lio_listio` operation fails asynchronously, the future will now
   include final error status for all failed operations.

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
