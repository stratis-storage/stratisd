A set of tests for stratisd D-Bus layer
==============================================

This code is not stable and is not intended for public use. Not only may
it change without notice, but it may also be entirely removed without notice.

Testing
-------
The bulk of the tests are designed to test the stratisd engine via the
D-Bus API. They test basic functionality and behavior of the various D-Bus
methods.

It is necessary to run these tests as root, since root permissions are
required to start stratisd.

To run the D-Bus tests, ensure that your PYTHONPATH includes the
src directory, set the environment variable STRATISD, to the location of your
Stratis executable, and: ::

    > export PYTHONPATH=src:../../../dbus-client-gen/src:../../../\
dbus-python-client-gen/src:../../../into-dbus-python/src:../../../\
dbus-signature-pyparsing/src
    > export STRATISD=../../target/debug/stratisd
    > make dbus-tests

Contributing
------------
Issues suggesting tests or pull requests that extend the existing test suite
are welcome.

The code style standard is PEP8.  Travis CI runs a compliance test on
all pull requests via yapf.  Please auto format code before opening a pull 
request via "make fmt".
