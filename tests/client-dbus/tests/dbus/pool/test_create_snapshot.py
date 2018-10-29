# Copyright 2018 Red Hat, Inc.
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
Test creating a snapshot
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


class CreateSnapshotTestCase(unittest.TestCase):
    """
    Test with an empty pool.
    """

    _POOLNAME = 'deadpool'
    _VOLNAME = 'some_fs'
    _SNAPSHOTNAME = 'ss_fs'

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
                'devices': self._devs
            })
        self._pool_object = get_object(poolpath)
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

        (fs_objects, rc, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {'specs': [self._VOLNAME]})

        self.assertEqual(rc, StratisdErrors.OK)

        fs_object_path = fs_objects[0][0]
        self.assertNotEqual(fs_object_path, "/")

        self._fs_object_path = fs_object_path

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testCreate(self):
        """
        Test creating a snapshot and ensure that it works.
        """

        (ss_object_path, rc, _) = Pool.Methods.SnapshotFilesystem(
            self._pool_object, {
                'origin': self._fs_object_path,
                'snapshot_name': self._SNAPSHOTNAME
            })

        self.assertEqual(rc, StratisdErrors.OK)
        self.assertNotEqual(ss_object_path, "/")

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {}))
        self.assertEqual(len([x for x in result]), 2)

    def testDuplicateSnapshotName(self):
        """
        Test creating a snapshot with duplicate name.
        """

        (ss_object_path, rc, _) = Pool.Methods.SnapshotFilesystem(
            self._pool_object, {
                'origin': self._fs_object_path,
                'snapshot_name': self._SNAPSHOTNAME
            })

        self.assertEqual(rc, StratisdErrors.OK)
        self.assertNotEqual(ss_object_path, "/")

        (ss_object_path_dupe_name, rc, _) = Pool.Methods.SnapshotFilesystem(
            self._pool_object, {
                'origin': self._fs_object_path,
                'snapshot_name': self._SNAPSHOTNAME
            })

        self.assertEqual(rc, StratisdErrors.ALREADY_EXISTS)
        self.assertEqual(ss_object_path_dupe_name, "/")

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {}))
        self.assertEqual(len([x for x in result]), 2)
