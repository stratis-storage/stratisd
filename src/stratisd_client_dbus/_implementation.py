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
    _INPUT_SIGS = abc.abstractproperty(doc="map from method name to data")
    _XFORMERS = abc.abstractproperty(doc="map from method name to xformer")

    @classmethod
    def callMethod(cls, proxy_object, method_name, *args):
        """
        Call a dbus method on a proxy object.

        :param proxy_object: the proxy object to invoke the method on
        :param method_name: a method name
        :param args: the arguments to pass to the dbus method

        :returns: the result of the call
        :rtype: object * int * str

        This method intentionally permits lower-level exceptions to be
        propagated.
        """
        xformed_args = cls._XFORMERS[method_name](args)
        dbus_method = getattr(proxy_object, method_name)
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
           name,
           dbus_interface=dbus.PROPERTIES_IFACE
        )


class Manager(Interface):
    """
    Manager interface.
    """

    _INTERFACE_NAME = 'org.storage.stratis1.Manager'

    _INPUT_SIGS = {
        "ConfigureSimulator" : "u",
        "CreatePool" : "sqas",
        "DestroyPool" : "s",
        "GetCacheObjectPath" : "s",
        "GetDevObjectPath" : "s",
        "GetDevTypes" : "",
        "GetErrorCodes" : "",
        "GetFilesystemObjectPath" : "ss",
        "GetPoolObjectPath" : "s",
        "GetRaidLevels" : "",
        "ListPools" : "",
    }
    _XFORMERS = _xformers(_INPUT_SIGS)


class Pool(Interface):
    """
    Pool interface.
    """

    _INTERFACE_NAME = 'org.storage.stratis1.pool'

    _INPUT_SIGS = {
       "AddCacheDevs": "as",
       "AddDevs": "as",
       "CreateFilesystems": "a(sst)",
       "DestroyFilesystems": "as",
       "ListCacheDevs": "",
       "ListDevs": "",
       "ListFilesystems": "",
       "RemoveCacheDevs": "asi",
       "RemoveDevs": "asi"
    }
    _XFORMERS = _xformers(_INPUT_SIGS)
