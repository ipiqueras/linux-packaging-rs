# `debian-packaging` History

<!-- next-header -->

## Unreleased

Released on ReleaseDate.

* `Send` added to various traits that were previously just `Read`. (#25)
* Fixed version comparisons of tilde against other characters. (#27)
* MSRV 1.75 -> 1.88.
* Migrated from `xz2` crate to `liblzma` (#29)
* `mailparse` 0.15 -> 0.16.
* `strum` 0.26 -> 0.27.
* `thiserror` 1.0 -> 2.0.

## 0.19.0

Released on 2025-01-26.

### Breaking Changes

* **BREAKING**: S3 functionality migrated from Rusoto to official AWS SDK for Rust (`aws-sdk-s3`).
  - Replaced `rusoto_core` and `rusoto_s3` dependencies with `aws-config` and `aws-sdk-s3`
  - S3 client constructors now require async context and have changed signatures
  - `S3RepositoryClient::new()` is now async and uses default AWS configuration
  - `S3Writer::new()` is now async and uses default AWS configuration
  - Region detection and handling updated to use new AWS SDK patterns
  - Users upgrading will need to update their S3-related code to use async constructors

### New Features

* Added complete S3 repository reading support via `S3RepositoryClient` implementing `RepositoryRootReader`
* Added `reader_from_str_async()` function for better S3 URL handling with automatic region detection
* S3 repository copier now supports both reading from and writing to S3 repositories
* Enhanced CLI documentation for S3 support in `drt` tool

### Improvements

* Better error messages for S3 operations using structured AWS SDK error types
* Improved async handling throughout S3 implementation
* More robust region detection and automatic configuration
* Updated S3 examples and documentation for modern AWS SDK patterns

### Dependencies

* Added `aws-config` 1.1.7 (optional)
* Added `aws-sdk-s3` 1.17.0 (optional)
* Added `tokio-util` 0.7 with compat feature (optional)
* Removed `rusoto_core` and `rusoto_s3` dependencies

## 0.18.0

Released on 2024-11-02.

* Fixed compile error when building without the `http` feature.
* MSRV 1.70 -> 1.75.
* `tokio` is now an optional dependency and is dependent on the `http` feature.
* `async-std` 1.12 -> 1.13.
* `async-tar` 0.4 -> 0.5.
* `bytes` 1.5 -> 1.8.
* `libflate` 2.0 -> 2.1.
* `mailparse` 0.14 -> 0.15.
* `once_cell` 1.18 -> 1.20.
* `os_str_bytes` 6.6 -> 7.0.
* `pgp` 0.10 -> 0.14.
* `regex` 1.10 -> 1.11.
* `reqwest` 0.11 -> 0.12.
* `smallvec` 1.11 -> 1.13.
* `strum` 0.25 -> 0.26.
* `strum_macros` 0.25 -> 0.26.
* `tokio` 1.33 -> 1.41.
* `url` 2.4 -> 2.5.
* `tempfile` 3.8 -> 3.13.

## 0.17.0

Released on 2023-11-03.

* MSRV 1.62 -> 1.70.
* Package version lexical comparison reworked to avoid sorting.
* `.deb` tar archives now correctly encode directories as directory entries.
* Release files with `Contents*` files in the top-level directory are now
  parsed without error. The stored `component` field is now an
  `Option<T>` and various APIs taking a `component: &str` now take
  `Option<&str>` since components are now optional.
* Various package updates to latest versions.

## 0.16.0 and Earlier

* No changelog kept.
