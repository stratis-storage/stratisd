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
Test DestroyPool.
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

class Destroy1TestCase(unittest.TestCase):
    """
    Test 'destroy' on empty database.

    'destroy' should always succeed on an empty database.
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
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testExecution(self):
        """
        Destroy should succeed since there is nothing to pass to DestroyPool.
        """
        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        pool = next(pools(managed_objects, {'Name': self._POOLNAME}), None)
        self.assertIsNone(pool)

    def testBogusObjectPath(self):
        """
        Success should occur on a bogus object path.
        """
        (_, rc, _) = Manager.Methods.DestroyPool(self._proxy, {'pool': "/"})
        self.assertEqual(rc, StratisdErrors.OK)


class Destroy2TestCase(unittest.TestCase):
    """
    Test 'destroy' on database which contains the given pool and an unknown
    number of devices.
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
        Manager.Methods.CreatePool(
           self._proxy,
           {
              'name': self._POOLNAME,
              'redundancy': (True, 0),
              'force': False,
              'devices': _DEVICE_STRATEGY.example()
           }
        )
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testExecution(self):
        """
        The pool was just created, so it must always be possible to destroy it.
        """
        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (pool1, _) = next(pools(managed_objects, {'Name': self._POOLNAME}))

        (result, rc, _) = \
                Manager.Methods.DestroyPool(self._proxy, {'pool': pool1})

        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        pool2 = next(pools(managed_objects, {'Name': self._POOLNAME}), None)

        self.assertEqual(rc, StratisdErrors.OK)
        self.assertIsNone(pool2)
        self.assertTrue(result)


class Destroy3TestCase(unittest.TestCase):
    """
    Test 'destroy' on database which contains the given pool and a volume.
    """
    _POOLNAME = 'deadpool'
    _VOLNAME = 'vol'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        Create a pool and a filesystem.
        """
        self._service = Service()
        self._service.setUp()

        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        ((poolpath, _), _, _) = Manager.Methods.CreatePool(
           self._proxy,
           {
              'name': self._POOLNAME,
              'redundancy': (True, 0),
              'force': False,
              'devices': _DEVICE_STRATEGY.example()
           }
        )
        Pool.Methods.CreateFilesystems(
           get_object(poolpath),
           {'specs': [self._VOLNAME]}
        )
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testExecution(self):
        """
        This should fail since the pool has a filesystem on it.
        """
        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (pool, _) = next(pools(managed_objects, {'Name': self._POOLNAME}))

        (result, rc, _) = \
           Manager.Methods.DestroyPool(self._proxy, {'pool': pool})
        self.assertEqual(rc, StratisdErrors.BUSY)
        self.assertEqual(result, False)

        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (pool1, _) = next(pools(managed_objects, {'Name': self._POOLNAME}))
        self.assertEqual(pool, pool1)


class Destroy4TestCase(unittest.TestCase):
    """
    Test 'destroy' on database which contains the given pool with no devices.
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
        Manager.Methods.CreatePool(
           self._proxy,
           {
              'name': self._POOLNAME,
              'redundancy': (True, 0),
              'force': False,
              'devices': []
           }
        )
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testExecution(self):
        """
        The pool was just created and has no devices. It should always be
        possible to destroy it.
        """
        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (pool, _) = next(pools(managed_objects, {'Name': self._POOLNAME}))

        (result, rc, _) = \
           Manager.Methods.DestroyPool(self._proxy, {'pool': pool})

        self.assertEqual(rc, StratisdErrors.OK)
        self.assertEqual(result, True)

        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        self.assertIsNone(
           next(pools(managed_objects, {'Name': self._POOLNAME}), None)
        )
