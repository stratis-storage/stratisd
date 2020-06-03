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


class CreateFSTestCase(SimTestCase):
    """
    Test with an empty pool.
    """

    _POOLNAME = "deadpool"
    _FSNAME = "fs"

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

    def test_create(self):
        """
        Test calling with no actual volume specification. An empty volume
        list should always succeed, and it should not increase the
        number of volumes.
        """
        ((is_some, result), return_code, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {"specs": []}
        )

        self.assertFalse(is_some)
        self.assertEqual(len(result), 0)
        self.assertEqual(return_code, StratisdErrors.OK)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )
        self.assertEqual(len(list(result)), 0)


class CreateFSTestCase1(SimTestCase):
    """
    Make a filesystem for the pool.
    """

    _POOLNAME = "deadpool"
    _FSNAME = "thunk"

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
        Pool.Methods.CreateFilesystems(self._pool_object, {"specs": [self._FSNAME]})

    def test_create(self):
        """
        Test calling by specifying a volume name. Because there is already
        a volume with the given name, the creation of the new volume should
        return the volume information as unchanged, and no additional volume
        should be created.
        """
        ((is_some, result), return_code, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {"specs": [self._FSNAME]}
        )

        self.assertEqual(return_code, StratisdErrors.OK)
        self.assertFalse(is_some)
        self.assertEqual(len(result), 0)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )
        self.assertEqual(len(list(result)), 1)

    def test_create_one(self):
        """
        Test calling by specifying a new and different volume name.
        The new volume will be created.
        """
        new_name = "newname"

        ((is_some, result), return_code, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {"specs": [new_name]}
        )

        self.assertEqual(return_code, StratisdErrors.OK)
        self.assertTrue(is_some)
        self.assertEqual(len(result), 1)

        (_, fs_name) = result[0]
        self.assertEqual(fs_name, new_name)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )
        self.assertEqual(len(list(result)), 2)

    def test_create_multiple(self):
        """
        Test calling by specifying multiple volume names.  Currently multiple
        volume names are not supported due to possible d-bus timeouts.  When
        multiple volume support is added back - this test should be removed.
        """
        ((is_some, result), return_code, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {"specs": ["a", "b"]}
        )

        self.assertEqual(return_code, StratisdErrors.ERROR)
        self.assertFalse(is_some)
        self.assertEqual(len(result), 0)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )
        self.assertEqual(len(list(result)), 1)
