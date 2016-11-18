A Python Wrapper for Stratisd D-Bus Calls
=========================================

This library is not stable and is not intended for public use. Not only may
it change without notice, but it may also be entirely removed without notice.

Introduction
------------
It is a simple wrapper for use by a client of the stratisd D-Bus API.
It ensures that the values placed on the dbus by the client conform to the
expected types.

Testing
-------
The source code of this library is simple and very short. Some tests exist
to verify that certain class-invariants are maintained.

The bulk of the tests in the repository test the stratisd engine via the
D-Bus API. They test basic functionality and behavior of the various D-Bus
methods and ensure that the values returned conform to their signature
specification.

To run the existing D-Bus tests, ensure that your PYTHONPATH includes the
src directory and then: ::

    > make dbus-tests

Contributing
------------
This is the rare library where the tests are actually more important than
the source code. Issues suggesting tests or pull requests that extend the
existing test suite are welcome.
