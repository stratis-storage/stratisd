# Copyright 2016 Red Hat, Inc.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""
Classes to implement dbus interface.
"""

import abc
import enum
import types

import dbus

from into_dbus_python import xformers

_IDENTITY = lambda x: x # pragma: no cover

def _option_to_tuple(value, default):
    """
    Converts a Python "option" type, None or a value, to a tuple.

    :param object value: any value
    :param object default: a valid default to use if value is None
    :returns: a tuple encoding the meaning of value
    :rtypes: bool * object
    """
    return (False, default) if value is None else (True, value)


def _info_to_xformer(specs):
    """
    Function that yields a xformer function.

    :param specs: specifications for this function
    :type names: iterable of triples, name, inxform function, sig
    :return: a transformation function
    :rtype: (list of object) -> (list of object)
    """
    inxforms = [y for (_, y, _) in specs]
    outxforms = [f for (f, _) in xformers("".join(z for (_, _, z) in specs))]
    expected_length = len(specs)

    def xformer(objects):
        """
        Transforms a list of objects to dbus-python types.

        :param objects: list of objects
        :type objects: list of object
        :returns: a transformed list of objects
        :rtype: list of object
        """
        if len(objects) != expected_length:
            raise ValueError("wrong number of objects")

        return \
           [x for (x, _) in (f(g(a)) for \
           ((f, g), a) in zip(zip(outxforms, inxforms), objects))]

    return xformer


def _xformers(key_to_sig):
    """
    Get a map from keys to functions from a map of names to signatures.

    :param key_to_sig: a map from keys to signatures
    :type key_to_sig: dict of object * str
    :returns: a map from keys to functions
    :rtype: dict of object * xformation function
    """
    return dict(
       (method, ([n for (n, _, _) in specs], _info_to_xformer(specs))) for \
       (method, specs) in key_to_sig.items())


class InterfaceSpec(abc.ABC):
    """
    Parent class for an interface hierarchy.
    """
    # pylint: disable=too-few-public-methods

    INTERFACE_NAME = abc.abstractproperty(doc="interface name")
    INPUT_SIGS = \
       abc.abstractproperty(doc="map from method name to input signatures")
    OUTPUT_SIGS = \
       abc.abstractproperty(doc="map from method name to output signatures")
    XFORMERS = abc.abstractproperty(doc="map from method name to xformer")
    PROPERTY_SIGS = abc.abstractproperty(doc="map from property names to sigs")


class ObjectManagerSpec(InterfaceSpec):
    """
    org.freedesktop.DBus.ObjectManager interface
    """
    # pylint: disable=too-few-public-methods

    class MethodNames(enum.Enum):
        """
        Method names.
        """
        GetManagedObjects = "GetManagedObjects"

    class PropertyNames(enum.Enum):
        """
        Property names.
        """
        pass

    INTERFACE_NAME = "org.freedesktop.DBus.ObjectManager"

    INPUT_SIGS = {
       MethodNames.GetManagedObjects: (),
    }

    OUTPUT_SIGS = {
       MethodNames.GetManagedObjects: "a{oa{sa{sv}}}"
    }
    XFORMERS = _xformers(INPUT_SIGS)

    PROPERTY_SIGS = {}


class FilesystemSpec(InterfaceSpec):
    """
    Filesystem interface.
    """
    # pylint: disable=too-few-public-methods

    class MethodNames(enum.Enum):
        """
        Names of the methods of the Filesystem class.
        """
        CreateSnapshot = "CreateSnapshot"
        SetName = "SetName"

    class PropertyNames(enum.Enum):
        """
        Names of the properties of the Filesystem interface.
        """
        Name = "Name"
        Pool = "Pool"
        Uuid = "Uuid"

    INTERFACE_NAME = 'org.storage.stratis1.filesystem'

    INPUT_SIGS = {
       MethodNames.CreateSnapshot: (("name", _IDENTITY, "s"),),
       MethodNames.SetName: (("name", _IDENTITY, "s"),),
    }
    OUTPUT_SIGS = {
       MethodNames.CreateSnapshot: "oqs",
       MethodNames.SetName: "bqs",
    }
    XFORMERS = _xformers(INPUT_SIGS)

    PROPERTY_SIGS = {
       PropertyNames.Name: "s",
       PropertyNames.Pool: "o",
       PropertyNames.Uuid: "s",
    }


class ManagerSpec(InterfaceSpec):
    """
    Manager interface.
    """
    # pylint: disable=too-few-public-methods

    class MethodNames(enum.Enum):
        """
        The method names of the manager interface.
        """
        ConfigureSimulator = "ConfigureSimulator"
        CreatePool = "CreatePool"
        DestroyPool = "DestroyPool"

    class PropertyNames(enum.Enum):
        """
        Names of the properties of the manager interface.
        """
        ErrorValues = "ErrorValues"
        RedundancyValues = "RedundancyValues"
        Version = "Version"

    INTERFACE_NAME = 'org.storage.stratis1.Manager'

    INPUT_SIGS = { # pragma: no cover
        MethodNames.ConfigureSimulator : (("denominator", _IDENTITY, "u"),),
        MethodNames.CreatePool :
           (
              ("name", _IDENTITY, "s"),
              ("redundancy", (lambda x: _option_to_tuple(x, 0)), "(bq)"),
              ("force", _IDENTITY, "b"),
              ("devices", _IDENTITY, "as"),
           ),
        MethodNames.DestroyPool : (("pool_object_path", _IDENTITY, "o"),),
    }
    OUTPUT_SIGS = {
        MethodNames.ConfigureSimulator : "qs",
        MethodNames.CreatePool : "(oas)qs",
        MethodNames.DestroyPool : "bqs",
    }
    XFORMERS = _xformers(INPUT_SIGS)

    PROPERTY_SIGS = {
       PropertyNames.ErrorValues: "a(sq)",
       PropertyNames.RedundancyValues: "a(sq)",
       PropertyNames.Version: "s"
    }


class PoolSpec(InterfaceSpec):
    """
    Pool interface.
    """
    # pylint: disable=too-few-public-methods

    class MethodNames(enum.Enum):
        """
        Names of the methods of the Pool class.
        """
        AddDevs = "AddDevs"
        CreateFilesystems = "CreateFilesystems"
        DestroyFilesystems = "DestroyFilesystems"
        SetName = "SetName"

    class PropertyNames(enum.Enum):
        """
        Names of the properties of the manager interface.
        """
        Name = "Name"
        Uuid = "Uuid"

    INTERFACE_NAME = 'org.storage.stratis1.pool'

    INPUT_SIGS = { # pragma: no cover
       MethodNames.AddDevs:
          (("force", _IDENTITY, "b"), ("devices", _IDENTITY, "as"),),
       MethodNames.CreateFilesystems: (("specs", _IDENTITY, "as"),),
       MethodNames.DestroyFilesystems: (("filesystems", _IDENTITY, "ao"),),
       MethodNames.SetName: (("new_name", _IDENTITY, "s"),)
    }
    OUTPUT_SIGS = {
       MethodNames.AddDevs: "asqs",
       MethodNames.CreateFilesystems: "a(os)qs",
       MethodNames.DestroyFilesystems: "asqs",
       MethodNames.SetName: "bqs"
    }
    XFORMERS = _xformers(INPUT_SIGS)

    PROPERTY_SIGS = {
       PropertyNames.Name: "s",
       PropertyNames.Uuid: "s",
    }


def _prop_builder(spec):
    """
    Returns a function that builds a property interface based on 'spec'.

    :param spec: the interface specification
    :type spec: type, a subtype of InterfaceSpec
    """

    def builder(namespace):
        """
        The property class's namespace.

        :param namespace: the class's namespace
        """

        def build_property(prop): # pragma: no cover
            """
            Build a single property getter for this class.

            :param prop: the property
            """

            def dbus_func(proxy_object): # pragma: no cover
                """
                The property getter.
                """
                return proxy_object.Get(
                   spec.INTERFACE_NAME,
                   prop.name,
                   dbus_interface=dbus.PROPERTIES_IFACE
                )

            return dbus_func

        for prop in spec.PropertyNames:
            namespace[prop.name] = staticmethod(build_property(prop)) # pragma: no cover

    return builder


def _iface_builder(spec):
    """
    Returns a function that builds a method interface based on 'spec'.

    :param spec: the interface specification
    :type spec: type, a subtype of InterfaceSpec
    """

    def builder(namespace):
        """
        Builds the class.

        :param namespace: the class's namespace
        """

        def build_method(method):
            """
            Build a single method for this class.

            :param method: the method
            """
            (names, func) = spec.XFORMERS[method]

            def dbus_func(proxy_object, **kwargs): # pragma: no cover
                """
                The function method spec.
                """
                if frozenset(names) != frozenset(kwargs.keys()):
                    raise ValueError("Bad keys")
                args = \
                   [v for (k, v) in \
                   sorted(kwargs.items(), key=lambda x: names.index(x[0]))]
                xformed_args = func(args)
                dbus_method = getattr(proxy_object, method.name)
                return dbus_method(
                   *xformed_args,
                   dbus_interface=spec.INTERFACE_NAME
                )

            return dbus_func

        for method in spec.MethodNames:
            namespace[method.name] = staticmethod(build_method(method))

        namespace['Properties'] = \
           types.new_class(
              "Properties",
              bases=(object,),
              exec_body=_prop_builder(spec)
           )

    return builder


ObjectManager = types.new_class(
   "ObjectManager",
   bases=(object,),
   exec_body=_iface_builder(ObjectManagerSpec)
)
Filesystem = types.new_class(
   "Filesystem",
   bases=(object,),
   exec_body=_iface_builder(FilesystemSpec)
)
Manager = types.new_class(
   "Manager",
   bases=(object,),
   exec_body=_iface_builder(ManagerSpec)
)
Pool = \
   types.new_class("Pool", bases=(object,), exec_body=_iface_builder(PoolSpec))
