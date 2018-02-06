A Library for Testing the stratisd D-Bus layer 
==============================================

This library is not stable and is not intended for public use. Not only may
it change without notice, but it may also be entirely removed without notice.

Testing
-------
The bulk of the tests in the repository test the stratisd engine via the
D-Bus API. They test basic functionality and behavior of the various D-Bus
methods.

It is necessary to run these tests as root, since root permissions are
required to start stratisd.

To run the existing D-Bus tests, ensure that your PYTHONPATH includes the
src directory, set the environment variable STRATISD, to the location of your
Stratis executable, and: ::

    > export PYTHONPATH=src:../../../dbus-client-gen/src:../../../\
dbus-python-client-gen/src:../../../into-dbus-python/src:../../../\
dbus-signature-pyparsing/src
    > export STRATISD=../../target/debug/stratisd
    > make dbus-tests

Contributing
------------
This is the rare library where the tests are actually more important than
the source code. Issues suggesting tests or pull requests that extend the
existing test suite are welcome.
