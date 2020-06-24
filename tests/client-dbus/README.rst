A set of Python-based tests for testing stratisd
================================================

This code is not stable and is not intended for public use. Not only may
it change without notice, but it may also be entirely removed without notice.

Testing
-------
The existing tests are divided into two categories:

* Tests that exercise the stratisd udev functionality using the real engine.
  These tests have a significant effect on the environment as they
  construct loopbacked devices, place signatures on them, and so forth.

* Tests that do miscellaneous things.

It is necessary to run all these tests as root, since root permissions are
required to start stratisd.

To run the tests, ensure that your PYTHONPATH includes the
src directory, set the environment variable STRATISD, to the location of your
Stratis executable, and: ::

    > export PYTHONPATH=src:../../../dbus-client-gen/src:../../../\
dbus-python-client-gen/src:../../../into-dbus-python/src:../../../\
dbus-signature-pyparsing/src
    > export STRATISD=../../target/debug/stratisd
    > make tests

To run only the udev tests: ::
   > make udev-tests

To run only the miscellaneous tests: ::
   > make misc-tests

Contributing
------------
Issues suggesting tests or pull requests that extend the existing test suite
are welcome.

The code style standard is PEP8.  Travis CI runs a compliance test on
all pull requests via black.  Please auto format code before opening a pull
request via "make fmt".
