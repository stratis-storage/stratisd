# Unsafe Unit Tests for stratisd

A collection of tests intended to test the different layers of stratis

## Running the tests

It is possible to run many tests using loopbacked devices to simulate real
devices. It is also possible to run these tests using real devices specified
in a configuration file. Both sorts of tests require root permissions and make
changes to your storage configuration. It is recommended that you run these
tests only on a dedicated test machine. Running these tests requires that the
user can sudo as root.

To run the loopbacked devices tests (in root of source tree):
```bash
 $ make test-loop
```

This runs all the tests that are enabled for loopbacked devices using
the Rust integration test framework.

To run the real device backed tests:

First, set up the configuration file to specify your set of scratch devices.
Copy `tests/test_config.json.example` to `tests/test_config.json` and modify
ok_to_destroy_dev_array_key to have a list of paths to scratch block
devices. For example:

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

Then (in root of source tree):
```bash
$ make test-real
```
