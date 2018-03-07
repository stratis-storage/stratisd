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
Test adding blockdevs to a pool.
"""

import time
import unittest

from stratisd_client_dbus import Manager
from stratisd_client_dbus import ObjectManager
from stratisd_client_dbus import Pool
from stratisd_client_dbus import StratisdErrors
from stratisd_client_dbus import blockdevs
from stratisd_client_dbus import get_object
from stratisd_client_dbus import pools

from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import _device_list
from .._misc import Service

_DEVICE_STRATEGY = _device_list(1)


class AddCacheDevsTestCase1(unittest.TestCase):
    """
    Test adding cachedevs to a pool which is initially empty.
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
        ((poolpath, _), _, _) = Manager.Methods.CreatePool(
           self._proxy,
           {
              'name': self._POOLNAME,
              'redundancy': (True, 0),
              'force': False,
              'devices': []
           }
        )
        self._pool_object = get_object(poolpath)
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testEmptyDevs(self):
        """
        Adding an empty list of cache devs should have no effect.
        """
        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (pool, _) = next(pools(managed_objects, {'Name': self._POOLNAME}))

        (result, rc, _) = Pool.Methods.AddCacheDevs(
           self._pool_object,
           {
              'force': False,
              'devices': []
           }
        )

        self.assertEqual(len(result), 0)
        self.assertEqual(rc, StratisdErrors.OK)

        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        blockdevs2 = blockdevs(managed_objects, {'Pool': pool})
        self.assertEqual(list(blockdevs2), [])

        blockdevs3 = blockdevs(managed_objects, {})
        self.assertEqual(list(blockdevs3), [])

    def testSomeDevs(self):
        """
        Adding a non-empty list of cache devs should succeed.
        """
        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (pool, _) = next(pools(managed_objects, {'Name': self._POOLNAME}))

        (result, rc, _) = Pool.Methods.AddCacheDevs(
           self._pool_object,
           {
              'force': False,
              'devices': _DEVICE_STRATEGY.example()
           }
        )

        num_devices_added = len(result)
        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})

        if rc == StratisdErrors.OK:
            self.assertGreater(num_devices_added, 0)
        else:
            self.assertEqual(num_devices_added, 0)

        blockdev_object_paths = frozenset(result)

        # blockdevs exported on the D-Bus are exactly those added
        blockdevs2 = list(blockdevs(managed_objects, {'Pool': pool}))
        blockdevs2_object_paths = frozenset([op for (op, _) in blockdevs2])
        self.assertEqual(blockdevs2_object_paths, blockdev_object_paths)

        # no duplicates in the object paths
        self.assertEqual(len(blockdevs2), num_devices_added)

        # There are no blockdevs but for those in this pool
        blockdevs3 = blockdevs(managed_objects, {})
        self.assertEqual(len(list(blockdevs3)), num_devices_added)

        # There are no datadevs belonging to this pool
        blockdevs4 = blockdevs(managed_objects, {'Pool': pool, 'Tier':  0})
        self.assertEqual(list(blockdevs4), [])


class AddCacheDevsTestCase2(unittest.TestCase):
    """
    Test adding devices to a pool which has some data devices.
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
        ((poolpath, devpaths), _, _) = Manager.Methods.CreatePool(
           self._proxy,
           {
              'name': self._POOLNAME,
              'redundancy': (True, 0),
              'force': False,
              'devices': _DEVICE_STRATEGY.example()
           }
        )
        self._pool_object = get_object(poolpath)
        self._devpaths = frozenset(devpaths)
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testEmptyDevs(self):
        """
        Adding an empty list of cache devs should have no effect.
        """
        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (pool, _) = next(pools(managed_objects, {'Name': self._POOLNAME}))

        blockdevs1 = blockdevs(managed_objects, {'Pool': pool, 'Tier': 0})
        self.assertEqual(self._devpaths, frozenset(op for (op, _) in blockdevs1))
        blockdevs2 = blockdevs(managed_objects, {'Pool': pool, 'Tier': 1})
        self.assertEqual(list(blockdevs2), [])

        (result, rc, _) = Pool.Methods.AddCacheDevs(
           self._pool_object,
           {
              'force': False,
              'devices': []
           }
        )

        self.assertEqual(len(result), 0)
        self.assertEqual(rc, StratisdErrors.OK)

        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        blockdevs3 = blockdevs(managed_objects, {'Pool': pool})
        self.assertEqual(frozenset(op for (op, _) in blockdevs3), self._devpaths)

    def testSomeDevs(self):
        """
        Adding a non-empty list of cache devs should succeed.
        """
        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (pool, _) = next(pools(managed_objects, {'Name': self._POOLNAME}))

        blockdevs1 = blockdevs(managed_objects, {'Pool': pool, 'Tier': 0})
        self.assertEqual(self._devpaths, frozenset(op for (op, _) in blockdevs1))
        (result, rc, _) = Pool.Methods.AddCacheDevs(
           self._pool_object,
           {
              'force': False,
              'devices': _DEVICE_STRATEGY.example()
           }
        )

        num_devices_added = len(result)
        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})

        if rc == StratisdErrors.OK:
            self.assertGreater(num_devices_added, 0)
        else:
            self.assertEqual(num_devices_added, 0)

        blockdev_object_paths = frozenset(result)

        # cache blockdevs exported on the D-Bus are exactly those added
        blockdevs2 = list(blockdevs(managed_objects, {'Pool': pool, 'Tier': 1}))
        self.assertEqual(
           frozenset(op for (op, _) in blockdevs2),
           blockdev_object_paths
        )

        # no duplicates in the object paths
        self.assertEqual(len(blockdevs2), num_devices_added)

        # There are no blockdevs but for those in this pool
        blockdevs3 = blockdevs(managed_objects, {'Pool': pool})
        blockdevs4 = blockdevs(managed_objects, {})
        self.assertEqual(len(list(blockdevs3)), len(list(blockdevs4)))

        # The number of datadevs has remained the same
        blockdevs5 = blockdevs(managed_objects, {'Pool': pool, 'Tier': 0})
        self.assertEqual(frozenset(op for (op, _) in blockdevs5), self._devpaths)
