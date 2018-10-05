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
Test creating a filesystem in a pool.
"""

import unittest

from stratisd_client_dbus import Manager
from stratisd_client_dbus import ObjectManager
from stratisd_client_dbus import Pool
from stratisd_client_dbus import StratisdErrors
from stratisd_client_dbus import filesystems
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import _device_list
from .._misc import Service

_DEVICE_STRATEGY = _device_list(0)


class CreateFSTestCase(unittest.TestCase):
    """
    Test with an empty pool.
    """

    _POOLNAME = 'deadpool'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        self._proxy = get_object(TOP_OBJECT)
        self._devs = _DEVICE_STRATEGY.example()
        ((poolpath, _), _, _) = Manager.Methods.CreatePool(
            self._proxy, {
                'name': self._POOLNAME,
                'redundancy': (True, 0),
                'force': False,
                'devices': self._devs
            })
        self._pool_object = get_object(poolpath)
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testCreate(self):
        """
        Test calling with no actual volume specification. An empty volume
        list should always succeed, and it should not increase the
        number of volumes.
        """
        (result, rc, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {'specs': []})

        self.assertEqual(len(result), 0)
        self.assertEqual(rc, StratisdErrors.OK)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {}))
        self.assertEqual(len([x for x in result]), 0)

    @unittest.skip("skip until creating multiple filesystems is supported")
    def testDuplicateSpecs(self):
        """
        Test calling with duplicate specification for same filesystem name.
        """
        new_name = "name"

        (result, rc, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {'specs': [new_name, new_name]})

        self.assertEqual(rc, StratisdErrors.OK)
        self.assertEqual(len(result), 1)

        (_, fs_name) = result[0]
        self.assertEqual(fs_name, new_name)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {}))
        self.assertEqual(len([x for x in result]), 1)


class CreateFSTestCase1(unittest.TestCase):
    """
    Make a filesystem for the pool.
    """

    _POOLNAME = 'deadpool'
    _VOLNAME = 'thunk'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        self._proxy = get_object(TOP_OBJECT)
        self._devs = _DEVICE_STRATEGY.example()
        ((poolpath, _), _, _) = Manager.Methods.CreatePool(
            self._proxy, {
                'name': self._POOLNAME,
                'redundancy': (True, 0),
                'force': False,
                'devices': self._devs
            })
        self._pool_object = get_object(poolpath)
        Pool.Methods.CreateFilesystems(self._pool_object,
                                       {'specs': [self._VOLNAME]})
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testCreate(self):
        """
        Test calling by specifying a volume name. Because there is already
        a volume with the given name, the creation of the new volume should
        fail, and no additional volume should be created.
        """
        (result, rc, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {'specs': [self._VOLNAME]})

        self.assertEqual(rc, StratisdErrors.ALREADY_EXISTS)
        self.assertEqual(len(result), 0)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {}))
        self.assertEqual(len([x for x in result]), 1)

    def testCreateOne(self):
        """
        Test calling by specifying a new and different volume name.
        The new volume will be created.
        """
        new_name = "newname"

        (result, rc, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {'specs': [new_name]})

        self.assertEqual(rc, StratisdErrors.OK)
        self.assertEqual(len(result), 1)

        (_, fs_name) = result[0]
        self.assertEqual(fs_name, new_name)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {}))
        self.assertEqual(len([x for x in result]), 2)

    @unittest.skip("skip until creating multiple filesystems is supported")
    def testCreateWithConflict(self):
        """
        Test calling by specifying several volumes. Because there is already
        a volume with the given name, the creation of the new volumes should
        fail, and no additional volume should be created.
        """
        (result, rc, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {'specs': [self._VOLNAME, "newname"]})

        self.assertEqual(rc, StratisdErrors.ALREADY_EXISTS)
        self.assertEqual(len(result), 0)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {}))
        self.assertEqual(len([x for x in result]), 1)

    def testCreateMultiple(self):
        """
        Test calling by specifying multiple volume names.  Currently multiple
        volume names are not supported due to possible d-bus timeouts.  When
        multiple volume support is added back - this test should be removed.
        """
        (result, rc, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {'specs': ["a", "b"]})

        self.assertEqual(rc, StratisdErrors.ERROR)
        self.assertEqual(len(result), 0)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {}))
        self.assertEqual(len([x for x in result]), 1)
