# Integration Tests for stratisd

A collection of tests intended to test the different layers of stratis

## Organization

Integration tests go in the stratisd/tests directory (unit tests go in each file
they're testing).

## Running the tests

Modify `tests/test_config.json` ok_to_destroy_dev_array_key to have a list of
paths to scratch block devices. For example:

```
{
    "ok_to_destroy_dev_array_key": [
    				   "/dev/vdb",
    				   "/dev/vdc",
    				   "/dev/vdd"
    ]
}
```

JSON requires commas between list items, but make sure to omit the comma after
the last list item.

Then, run `cargo test` as root. (Root permissions are needed to access block
devices, and to create device-mapper targets.)

## Logging

Integration test logging is done via the crate env_logger.  See : 
https://doc.rust-lang.org/log/env_logger/ for details.

Example: To enable logging for specific modules set:

RUST_LOG=blockdev_tests=debug,pool_tests=debug,util::test_results=debug

