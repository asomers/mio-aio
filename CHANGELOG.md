## [Unreleased] - ReleaseDate
### Added
- Support for submitting multiple operations at once with `lio_listio`.

### Changed
- `AioCb` structures can no longer be created from a `Rc<Box<[u8]>>`.  Use a
  `Box<Borrow<[u8]>>` or a `Box<BorrowMut<[u8]>>` instead.

### Fixed

### Removed
