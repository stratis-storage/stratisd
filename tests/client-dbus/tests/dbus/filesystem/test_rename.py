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
Test renaming a filesystem.
"""

import unittest

from stratisd_client_dbus import Filesystem
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


class SetNameTestCase(unittest.TestCase):
    """
    Set up a pool with a name and one filesystem.
    """

    _POOLNAME = 'deadpool'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._fs_name = 'fs'
        self._service = Service()
        self._service.setUp()
        self._proxy = get_object(TOP_OBJECT)
        ((self._pool_object_path, _), _, _) = Manager.Methods.CreatePool(
            self._proxy, {
                'name': self._POOLNAME,
                'redundancy': (True, 0),
                'force': False,
                'devices': _DEVICE_STRATEGY.example()
            })
        self._pool_object = get_object(self._pool_object_path)
        (created, _, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {'specs': [self._fs_name]})
        self._filesystem_object_path = created[0][0]
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
        filesystem = get_object(self._filesystem_object_path)
        (result, rc, _) = Filesystem.Methods.SetName(filesystem,
                                                     {'name': self._fs_name})

        self.assertEqual(rc, StratisdErrors.OK)
        self.assertFalse(result)

    def testNewName(self):
        """
        Test rename to new name.
        """
        filesystem = get_object(self._filesystem_object_path)
        (result, rc, _) = Filesystem.Methods.SetName(filesystem,
                                                     {'name': "new"})

        self.assertEqual(rc, StratisdErrors.OK)
        self.assertTrue(result)

        managed_objects = \
           ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (fs_object_path, _) = next(
            filesystems(props={
                'Name': 'new'
            }).search(managed_objects))
        self.assertEqual(self._filesystem_object_path, fs_object_path)

        fs_object_path = next(
            filesystems(props={
                'Name': self._fs_name
            }).search(managed_objects),
            None)
        self.assertIsNone(fs_object_path)
