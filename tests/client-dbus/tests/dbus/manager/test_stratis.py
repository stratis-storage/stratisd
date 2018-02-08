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
Test 'stratisd'.
"""

import time
import unittest

from dbus_python_client_gen import DPClientInvalidArgError

from stratisd_client_dbus import Manager
from stratisd_client_dbus import ObjectManager
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import Service


class StratisTestCase(unittest.TestCase):
    """
    Test meta information about stratisd.
    """

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testStratisVersion(self):
        """
        Getting version should succeed.

        Major version number should be 0.
        """
        version = Manager.Properties.Version.Get(get_object(TOP_OBJECT))
        (major, _, _) = version.split(".")
        self.assertEqual(major, "0")

class StratisTestCase2(unittest.TestCase):
    """
    Test exceptions raised by various errors.
    """

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testArguments(self):
        """
        Incorrect arguments should cause a type error.
        """
        with self.assertRaises(TypeError):
            Manager.Properties.Version.Get(get_object(TOP_OBJECT), {})

    def testFunctionName(self):
        """
        We know that it is impossible to set the Stratis version, so Set
        method should not exist, and this should result in an Attribute error.
        """
        with self.assertRaises(AttributeError):
            Manager.Properties.Version.Set(get_object(TOP_OBJECT), {})

    def testFunctionArgs(self):
        """
        If the arguments to the D-Bus method are incorrect, the exception is
        a DPClientInvalidArgError.

        Incorrectness can be caused by incorrect keyword args, but also
        by incorrect type of argument.
        """
        with self.assertRaises(DPClientInvalidArgError):
            ObjectManager.Methods.GetManagedObjects(self._proxy, {'bogus': 2})
        with self.assertRaises(DPClientInvalidArgError):
            Manager.Methods.DestroyPool(self._proxy, {'pool': 2})
