# Integration Tests for stratisd

A collection of tests intended to test the different layers of stratis

## Organization

Integration tests go in the stratisd/tests directory (unit tests go in each file
they're testing).

## Logging

Integration test logging is done via the crate env_logger.  See : 
https://doc.rust-lang.org/log/env_logger/ for details.

Example: To enable logging for specific modules set:

RUST_LOG=blockdev_tests=debug,pool_tests=debug,util::test_results=debug

