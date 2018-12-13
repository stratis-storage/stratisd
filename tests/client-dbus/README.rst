A set of tests for stratisd D-Bus layer
==============================================

This code is not stable and is not intended for public use. Not only may
it change without notice, but it may also be entirely removed without notice.

Testing
-------
The existing tests are divided into two categories:

* Tests that exercise the stratisd D-Bus layer using the sim engine. These
  tests test basic functionality of the D-Bus methods. The effect these
  tests have on the environment is that they start and stop the stratisd
  daemon and communicate with the daemon over D-Bus.

* Tests that exercise the stratisd udev functionality using the real engine.
  These tests have a more significant effect on the environment as they
  construct loopbacked devices, place signatures on them, and so forth.

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

To run only the D-Bus tests: ::
   > make dbus-tests

To run only the udev tests: ::
   > make udev-tests

Contributing
------------
Issues suggesting tests or pull requests that extend the existing test suite
are welcome.

The code style standard is PEP8.  Travis CI runs a compliance test on
all pull requests via yapf.  Please auto format code before opening a pull
request via "make fmt".
