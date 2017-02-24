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

import time
import unittest

from stratisd_client_dbus import Filesystem
from stratisd_client_dbus import Manager
from stratisd_client_dbus import Pool
from stratisd_client_dbus import StratisdErrorsGen
from stratisd_client_dbus import get_object
from stratisd_client_dbus import get_managed_objects

from stratisd_client_dbus._constants import TOP_OBJECT

from stratisd_client_dbus._implementation import FilesystemSpec
from stratisd_client_dbus._implementation import ManagerSpec
from stratisd_client_dbus._implementation import PoolSpec

from .._misc import checked_call
from .._misc import _device_list
from .._misc import Service

_FN = FilesystemSpec.MethodNames
_MN = ManagerSpec.MethodNames
_PN = PoolSpec.MethodNames

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
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        self._errors = StratisdErrorsGen.get_object()
        ((self._pool_object_path, _), _, _) = Manager.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=0,
           force=False,
           devices=_DEVICE_STRATEGY.example()
        )
        self._pool_object = get_object(self._pool_object_path)
        (filesystems, _, _) = Pool.CreateFilesystems(
           self._pool_object,
           specs=[self._fs_name]
        )
        self._filesystem_object_path = filesystems[0][0]
        Manager.ConfigureSimulator(self._proxy, denominator=8)

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
        (result, rc, _) = checked_call(
           Filesystem.SetName(filesystem, name=self._fs_name),
           FilesystemSpec.OUTPUT_SIGS[_FN.SetName]
        )

        self.assertEqual(rc, self._errors.OK)
        self.assertFalse(result)

    def testNewName(self):
        """
        Test rename to new name.
        """
        filesystem = get_object(self._filesystem_object_path)
        (result, rc, _) = checked_call(
           Filesystem.SetName(filesystem, name="new"),
           FilesystemSpec.OUTPUT_SIGS[_FN.SetName]
        )

        self.assertEqual(rc, self._errors.OK)
        self.assertTrue(result)

        managed_objects = get_managed_objects(self._proxy)
        (fs_object_path, _) = next(managed_objects.filesystems({'Name': 'new'}))
        self.assertEqual(self._filesystem_object_path, fs_object_path)

        fs_object_path = \
           next(managed_objects.filesystems({'Name': self._fs_name}), None)
        self.assertIsNone(fs_object_path)
