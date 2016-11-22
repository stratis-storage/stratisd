A Python Wrapper for Stratisd D-Bus Calls
=========================================

This library is not stable and is not intended for public use. Not only may
it change without notice, but it may also be entirely removed without notice.

Introduction
------------
This library is a simple wrapper for use by a client of the stratisd D-Bus API.
It is built on top of the dbus-python library. It ensures that the values
placed on the dbus by the client conform to the expected types.

Stratisd explicitly exposes the value of constants, like error codes, on the
dbus. This library has a facility for reading and cacheing these values,
so that clients can refer to these constants by name, rather than by their
value. This makes clients more robust to changes in the stratisd implementation,
i.e., reordering of the constants resulting in different numeric values for
their names. This also makes the stratisd implementation more easily
extensible, since new values can be added without forcing any changes in the
client. The class corresponding to a particular list of constants is
obtained by invoking the get_object() method of the corresponding Gen class. ::

    >>> STRATISD_ERRORS = StratisdErrorsGen.get_object()

The constants can then be accessed as attributes by name: ::

    >>> STRATISD_ERRORS.OK

It is up to the client to cache the generated class at whatever level is
appropriate.

Stratisd also implements certain special interfaces for the various objects
it exposes on the dbus. These are implemented as classes, which expose the
methods and properties of the interface as static attributes. For example, ::

    >>> Manager.GetPoolObjectPath(proxy_object, "apool")

invokes the GetPoolObjectPath method for the Manager interface on the proxy
object, passing as an argument the name of the pool. ::

    >>> Dev.Properties.Size(proxy_object)

obtains the value of the Size property of the Dev interface for proxy_object.
The arguments placed on the D-Bus for a given method are martialed according
to cached information about stratisd's input specification for that method
into the correct dbus-python types.

This library does not have its own set of exceptions. Instead, it raises
ValueError exceptions, or allows internal errors to propagate. None of these
exceptions can be considered part of its interface.

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
