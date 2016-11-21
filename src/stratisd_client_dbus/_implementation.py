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

import dbus

from into_dbus_python import xformer

def _xformers(key_to_sig):
    """
    Get a map from keys to functions from a map of names to signatures.

    :param key_to_sig: a map from keys to signatures
    :type key_to_sig: dict of object * str
    :returns: a map from keys to functions
    :rtype: dict of object * xformation function
    """
    sig_to_xformers = dict((sig, xformer(sig)) for sig in key_to_sig.values())
    return dict((method, sig_to_xformers[sig]) for \
       (method, sig) in key_to_sig.items())


class Interface(abc.ABC):
    """
    Parent class for an interface hierarchy.
    """

    _INTERFACE_NAME = abc.abstractproperty(doc="interface name")
    _INPUT_SIGS = \
       abc.abstractproperty(doc="map from method name to input signatures")
    _OUTPUT_SIGS = \
       abc.abstractproperty(doc="map from method name to output signatures")
    _XFORMERS = abc.abstractproperty(doc="map from method name to xformer")
    _PROPERTY_NAMES = abc.abstractproperty(doc="list of property names")

    @classmethod
    def callMethod(cls, proxy_object, method, *args):
        """
        Call a dbus method on a proxy object.

        :param proxy_object: the proxy object to invoke the method on
        :param method: a method name
        :param args: the arguments to pass to the dbus method

        :returns: the result of the call
        :rtype: object * int * str

        This method intentionally permits lower-level exceptions to be
        propagated.
        """
        xformed_args = cls._XFORMERS[method](args)
        dbus_method = getattr(proxy_object, method.name)
        return dbus_method(*xformed_args, dbus_interface=cls._INTERFACE_NAME)

    @classmethod
    def getProperty(cls, proxy_object, name):
        """
        Get a property with name 'name'.

        :param proxy_object: the proxy object
        :param str name: the name of the property

        :returns: the value of the property
        :rtype: object
        """
        return proxy_object.Get(
           cls._INTERFACE_NAME,
           name.name,
           dbus_interface=dbus.PROPERTIES_IFACE
        )


class Cache(Interface):
    """
    Cache device interface.
    """

    class MethodNames(enum.Enum):
        """
        Names of the methods of the dev interface.
        """
        pass

    class PropertyNames(enum.Enum):
        """
        Names of the properties of the Filesystem interface.
        """
        Size = "Size"

    _INTERFACE_NAME = 'org.storage.stratis1.cache'

    _INPUT_SIGS = {
    }
    _OUTPUT_SIGS = {
    }
    _XFORMERS = _xformers(_INPUT_SIGS)


class Dev(Interface):
    """
    Blockdev interface.
    """

    class MethodNames(enum.Enum):
        """
        Names of the methods of the dev interface.
        """
        pass

    class PropertyNames(enum.Enum):
        """
        Names of the properties of the Filesystem interface.
        """
        Size = "Size"

    _INTERFACE_NAME = 'org.storage.stratis1.dev'

    _INPUT_SIGS = {
    }
    _OUTPUT_SIGS = {
    }
    _XFORMERS = _xformers(_INPUT_SIGS)


class Filesystem(Interface):
    """
    Filesystem interface.
    """

    class MethodNames(enum.Enum):
        """
        Names of the methods of the Filesystem class.
        """
        CreateSnapshot = "CreateSnapshot"
        Rename = "Rename"
        SetMountpoint = "SetMountpoint"
        SetQuota = "SetQuota"

    class PropertyNames(enum.Enum):
        """
        Names of the properties of the Filesystem interface.
        """
        pass

    _INTERFACE_NAME = 'org.storage.stratis1.filesystem'

    _INPUT_SIGS = {
       MethodNames.CreateSnapshot: "s",
       MethodNames.Rename: "s",
       MethodNames.SetMountpoint: "",
       MethodNames.SetQuota: "s"
    }
    _OUTPUT_SIGS = {
       MethodNames.CreateSnapshot: "oqs",
       MethodNames.Rename: "oqs",
       MethodNames.SetMountpoint: "oqs",
       MethodNames.SetQuota: "oqs"
    }
    _XFORMERS = _xformers(_INPUT_SIGS)


class Manager(Interface):
    """
    Manager interface.
    """

    class MethodNames(enum.Enum):
        """
        The method names of the manager interface.
        """
        ConfigureSimulator = "ConfigureSimulator"
        CreatePool = "CreatePool"
        DestroyPool = "DestroyPool"
        GetCacheObjectPath = "GetCacheObjectPath"
        GetDevObjectPath = "GetDevObjectPath"
        GetDevTypes = "GetDevTypes"
        GetErrorCodes = "GetErrorCodes"
        GetFilesystemObjectPath = "GetFilesystemObjectPath"
        GetPoolObjectPath = "GetPoolObjectPath"
        GetRaidLevels = "GetRaidLevels"
        ListPools = "ListPools"

    class PropertyNames(enum.Enum):
        """
        Names of the properties of the manager interface.
        """
        pass

    _INTERFACE_NAME = 'org.storage.stratis1.Manager'

    _INPUT_SIGS = {
        MethodNames.ConfigureSimulator : "u",
        MethodNames.CreatePool : "sqbas",
        MethodNames.DestroyPool : "s",
        MethodNames.GetCacheObjectPath : "s",
        MethodNames.GetDevObjectPath : "s",
        MethodNames.GetDevTypes : "",
        MethodNames.GetErrorCodes : "",
        MethodNames.GetFilesystemObjectPath : "ss",
        MethodNames.GetPoolObjectPath : "s",
        MethodNames.GetRaidLevels : "",
        MethodNames.ListPools : "",
    }
    _OUTPUT_SIGS = {
        MethodNames.ConfigureSimulator : "qs",
        MethodNames.CreatePool : "oqs",
        MethodNames.DestroyPool : "qs",
        MethodNames.GetCacheObjectPath : "oqs",
        MethodNames.GetDevObjectPath : "oqs",
        MethodNames.GetDevTypes : "",
        MethodNames.GetErrorCodes : "a(sqs)",
        MethodNames.GetFilesystemObjectPath : "oqs",
        MethodNames.GetPoolObjectPath : "oqs",
        MethodNames.GetRaidLevels : "a(sqs)",
        MethodNames.ListPools : "asqs",
    }
    _XFORMERS = _xformers(_INPUT_SIGS)


class Pool(Interface):
    """
    Pool interface.
    """

    class MethodNames(enum.Enum):
        """
        Names of the methods of the Pool class.
        """
        AddCacheDevs = "AddCacheDevs"
        AddDevs = "AddDevs"
        CreateFilesystems = "CreateFilesystems"
        DestroyFilesystems = "DestroyFilesystems"
        ListCacheDevs = "ListCacheDevs"
        ListDevs = "ListDevs"
        ListFilesystems = "ListFilesystems"
        RemoveCacheDevs = "RemoveCacheDevs"
        RemoveDevs = "RemoveDevs"

    class PropertyNames(enum.Enum):
        """
        Names of the properties of the manager interface.
        """
        pass

    _INTERFACE_NAME = 'org.storage.stratis1.pool'

    _INPUT_SIGS = {
       MethodNames.AddCacheDevs: "as",
       MethodNames.AddDevs: "as",
       MethodNames.CreateFilesystems: "a(sst)",
       MethodNames.DestroyFilesystems: "as",
       MethodNames.ListCacheDevs: "",
       MethodNames.ListDevs: "",
       MethodNames.ListFilesystems: "",
       MethodNames.RemoveCacheDevs: "asi",
       MethodNames.RemoveDevs: "asi"
    }
    _OUTPUT_SIGS = {
       MethodNames.AddCacheDevs: "a(oqs)qs",
       MethodNames.AddDevs: "a(oqs)qs",
       MethodNames.CreateFilesystems: "a(oqs)qs",
       MethodNames.DestroyFilesystems: "a(qs)qs",
       MethodNames.ListCacheDevs: "asqs",
       MethodNames.ListDevs: "asqs",
       MethodNames.ListFilesystems: "asqs",
       MethodNames.RemoveCacheDevs: "a(qs)qs",
       MethodNames.RemoveDevs: "a(qs)qs"
    }
    _XFORMERS = _xformers(_INPUT_SIGS)
