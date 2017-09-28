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
Test renaming a pool.
"""

import time
import unittest

from stratisd_client_dbus import Manager
from stratisd_client_dbus import ObjectManager
from stratisd_client_dbus import Pool
from stratisd_client_dbus import StratisdErrors
from stratisd_client_dbus import get_object
from stratisd_client_dbus import pools

from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import _device_list
from .._misc import Service

_DEVICE_STRATEGY = _device_list(0)


class SetNameTestCase(unittest.TestCase):
    """
    Set up a pool with a name.
    """

    _POOLNAME = 'deadpool'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        ((self._pool_object_path, _), _, _) = Manager.Methods.CreatePool(
           self._proxy,
           {
              'name': self._POOLNAME,
              'redundancy': (True, 0),
              'force': False,
              'devices': _DEVICE_STRATEGY.example()
           }
        )
        self._pool_object = get_object(self._pool_object_path)
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testNullMapping(self):
        """
        Test rename to same name.
        """
        (result, rc, _) = Pool.Methods.SetName(
           self._pool_object,
           {'name': self._POOLNAME}
        )

        self.assertEqual(rc, StratisdErrors.OK)
        self.assertFalse(result)

        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        result = next(pools(managed_objects, {'Name': self._POOLNAME}), None)
        self.assertIsNotNone(result)
        (pool, _) = result
        self.assertEqual(pool, self._pool_object_path)

    def testNewName(self):
        """
        Test rename to new name.
        """
        new_name = "new"

        (result, rc, _) = Pool.Methods.SetName(
           self._pool_object,
           {'name': new_name}
        )

        self.assertTrue(result)
        self.assertEqual(rc, StratisdErrors.OK)

        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        self.assertIsNone(
           next(pools(managed_objects, {'Name': self._POOLNAME}), None)
        )
        result = next(pools(managed_objects, {'Name': new_name}), None)
        self.assertIsNotNone(result)
        (pool, _) = result
        self.assertEqual(pool, self._pool_object_path)
