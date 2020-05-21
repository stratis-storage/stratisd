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

# isort: LOCAL
from stratisd_client_dbus import (
    Manager,
    ObjectManager,
    Pool,
    StratisdErrors,
    filesystems,
    get_object,
)
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase, device_name_list

_DEVICE_STRATEGY = device_name_list(1)


class CreateSnapshotTestCase(SimTestCase):
    """
    Test with an empty pool.
    """

    _POOLNAME = "deadpool"
    _FSNAME = "some_fs"
    _SNAPSHOTNAME = "ss_fs"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        self._proxy = get_object(TOP_OBJECT)
        ((_, (poolpath, _)), _, _) = Manager.Methods.CreatePool(
            self._proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": _DEVICE_STRATEGY(),
            },
        )
        self._pool_object = get_object(poolpath)

        ((_, fs_objects), return_code, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {"specs": [self._FSNAME]}
        )

        self.assertEqual(return_code, StratisdErrors.OK)

        self._fs_object_path = fs_objects[0][0]
        self.assertNotEqual(self._fs_object_path, "/")

    def test_create(self):
        """
        Test creating a snapshot and ensure that it works.
        """

        ((is_some, ss_object_path), return_code, _) = Pool.Methods.SnapshotFilesystem(
            self._pool_object,
            {"origin": self._fs_object_path, "snapshot_name": self._SNAPSHOTNAME},
        )

        self.assertTrue(is_some)
        self.assertEqual(return_code, StratisdErrors.OK)
        self.assertNotEqual(ss_object_path, "/")

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )
        self.assertEqual(len(list(result)), 2)

    def test_duplicate_snapshot_name(self):
        """
        Test creating a snapshot with duplicate name.
        """

        ((is_some, ss_object_path), return_code, _) = Pool.Methods.SnapshotFilesystem(
            self._pool_object,
            {"origin": self._fs_object_path, "snapshot_name": self._SNAPSHOTNAME},
        )

        self.assertTrue(is_some)
        self.assertEqual(return_code, StratisdErrors.OK)
        self.assertNotEqual(ss_object_path, "/")

        (
            (is_some, ss_object_path_dupe_name),
            return_code,
            _,
        ) = Pool.Methods.SnapshotFilesystem(
            self._pool_object,
            {"origin": self._fs_object_path, "snapshot_name": self._SNAPSHOTNAME},
        )

        self.assertFalse(is_some)
        self.assertEqual(return_code, StratisdErrors.OK)
        self.assertEqual(ss_object_path_dupe_name, "/")

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )
        self.assertEqual(len(list(result)), 2)
