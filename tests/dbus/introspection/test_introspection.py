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
Test correctness of introspection data published by the service.
"""

import time

import xml.etree.ElementTree as ET

import unittest

import dbus

from stratisd_client_dbus import Manager
from stratisd_client_dbus import Pool
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from stratisd_client_dbus._implementation import FilesystemSpec
from stratisd_client_dbus._implementation import ManagerSpec
from stratisd_client_dbus._implementation import PoolSpec

from .._misc import Service

_SPEC_CLASSES = (FilesystemSpec, ManagerSpec, PoolSpec)

def _signature(method_data, direction):
    """
    Get the signature from the introspection data.

    :param ElementTree method_data: method data, as xml
    :param str direction: "in" or "out"

    :returns: the in or out signature
    :rtype: str
    """
    return \
       "".join(x.attrib['type'] for \
          x in method_data.findall("./arg[@direction='%s']" % direction))

def _verify_names(introspection_names, spec_names):
    """
    Verify that names are correct.

    :param iterable introspection_names:
    :param iterable spec_names:

    :returns: an error string if some error was found, else None
    :rtype: str or NoneType
    """
    introspection_names = [x for x in introspection_names]

    names_set = frozenset(introspection_names)
    if len(names_set) != len(introspection_names):
        return "duplicate_names"

    if names_set != frozenset(spec_names):
        return "names do not match"

    return None

def _verify_properties(klass, klass_datum):
    """
    Verify that property introspection data matches klass's expectations.

    :param type klass: the specification class for an interface
    :param str klass_datum: introspection data for this class

    :returns: an error string if some error was found, else None
    :rtype: str or NoneType
    """
    for prop in klass.PropertyNames:
        property_data = klass_datum.findall("./property[@name='%s']" % prop.name)
        property_datum = property_data[0]

        if property_datum.attrib['type'] != klass.PROPERTY_SIGS[prop]:
            return "signature for property %s in interface %s does not match" \
               % (prop.name, klass.INTERFACE_NAME)

        if property_datum.attrib['access'] != 'read':
            return "property %s in interface %s has access other than 'read'" \
               % (prop.name, klass.INTERFACE_NAME)

def _verify_methods(klass, klass_datum):
    """
    Verify that method introspection data matches klass's expectations.

    :param type klass: the specification class for an interface
    :param str klass_datum: introspection data for this class

    :returns: an error string if some error was found, else None
    :rtype: str or NoneType
    """
    for method in klass.MethodNames:
        method_data = klass_datum.findall("./method[@name='%s']" % method.name)
        method_datum = method_data[0]

        sig = _signature(method_datum, "in")
        if sig != "".join(s for (_, _, s) in klass.INPUT_SIGS[method]):
            return "in signatures for method %s in interface %s do not match" \
               % (method.name, klass.INTERFACE_NAME)

        sig = _signature(method_datum, "out")
        if sig != klass.OUTPUT_SIGS[method]:
            return "out signatures for method %s in interface %s do not match" \
               % (method.name, klass.INTERFACE_NAME)
    return None

def _verify_class(klass, introspect_data):
    """
    Verify that introspection data matches klass's expectations.

    :param type klass: the specification class for an interface
    :param str introspect_data: introspection data for an object

    :returns: an error string if some error was found, else None
    :rtype: str or NoneType
    """
    klass_data = \
       introspect_data.findall("./interface[@name='%s']" % klass.INTERFACE_NAME)
    klass_datum = klass_data[0]

    result = _verify_names(
       (m.attrib['name'] for m in klass_datum.findall("./method")),
       (n.name for n in klass.MethodNames)
    )
    if result is not None:
        return "%s for methods in %s introspection_data" \
           % (result, klass.INTERFACE_NAME)

    result = _verify_names(
       (m.attrib['name'] for m in klass_datum.findall("./property")),
       (n.name for n in klass.PropertyNames)
    )
    if result is not None:
        return "%s for properties in %s introspection_data" \
           % (result, klass.INTERFACE_NAME)

    result = _verify_methods(klass, klass_datum)
    if result is not None:
        return result

    return _verify_properties(klass, klass_datum)


class InterfacesTestCase(unittest.TestCase):
    """
    Test that information about interfaces published by Introspect matches
    what we expect.
    """

    def setUp(self):
        """
        Obtain the Introspect() xml.
        """
        self._introspection_data = dict()

        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        proxy = get_object(TOP_OBJECT)

        self._introspection_data[ManagerSpec.INTERFACE_NAME] = \
           proxy.Introspect(dbus_interface=dbus.INTROSPECTABLE_IFACE)

        ((poolpath, _), _, _) = Manager.CreatePool(
           proxy,
           name="name",
           redundancy=0,
           force=False,
           devices=[]
        )
        pool = get_object(poolpath)
        self._introspection_data[PoolSpec.INTERFACE_NAME] = \
           pool.Introspect(dbus_interface=dbus.INTROSPECTABLE_IFACE)

        ([(fspath, _)], _, _) = \
           Pool.CreateFilesystems(pool, specs=[("filesystem", '', None)])
        fs = get_object(fspath)
        self._introspection_data[FilesystemSpec.INTERFACE_NAME] = \
           fs.Introspect(dbus_interface=dbus.INTROSPECTABLE_IFACE)

    def tearDown(self):
        self._service.tearDown()

    def testInterfaces(self):
        """
        Test that *Spec data are correct.
        """
        for klass in _SPEC_CLASSES:
            result = _verify_class(
               klass,
               ET.fromstring(self._introspection_data[klass.INTERFACE_NAME])
            )
            self.assertIsNone(result)
