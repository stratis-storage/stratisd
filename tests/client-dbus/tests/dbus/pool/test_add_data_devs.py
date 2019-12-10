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

# isort: LOCAL
from stratisd_client_dbus import (
    Manager,
    ObjectManager,
    Pool,
    StratisdErrors,
    blockdevs,
    get_object,
    pools,
)
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase, device_name_list

_DEVICE_STRATEGY = device_name_list(1)


class AddDataDevsTestCase(SimTestCase):
    """
    Test adding devices to a pool which is initially empty.
    """

    _POOLNAME = "deadpool"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        self._proxy = get_object(TOP_OBJECT)
        self._data_devices = _DEVICE_STRATEGY()
        ((_, (poolpath, self._blockdev_paths)), _, _) = Manager.Methods.CreatePool(
            self._proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": self._data_devices,
            },
        )
        self._pool_object = get_object(poolpath)
        Manager.Methods.ConfigureSimulator(self._proxy, {"denominator": 8})

    def testEmptyDevs(self):
        """
        Adding an empty list of devs should fail.
        """
        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (pool, _) = next(pools(props={"Name": self._POOLNAME}).search(managed_objects))

        blockdevs1 = blockdevs(props={"Pool": pool}).search(managed_objects)
        self.assertEqual(len(list(blockdevs1)), len(self._data_devices))

        ((is_some, _), rc, _) = Pool.Methods.AddDataDevs(
            self._pool_object, {"devices": []}
        )

        self.assertFalse(is_some)
        self.assertEqual(rc, StratisdErrors.ERROR)

        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        blockdevs2 = blockdevs(props={"Pool": pool}).search(managed_objects)
        self.assertEqual(len(list(blockdevs2)), len(self._data_devices))

        blockdevs3 = blockdevs(props={}).search(managed_objects)
        self.assertEqual(len(list(blockdevs3)), len(self._data_devices))

    def testSomeDevs(self):
        """
        Adding a non-empty list of devs should increase the number of devs
        in the pool.
        """
        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (pool, _) = next(pools(props={"Name": self._POOLNAME}).search(managed_objects))

        blockdevs1 = blockdevs(props={"Pool": pool}).search(managed_objects)
        self.assertEqual(len(list(blockdevs1)), len(self._data_devices))

        ((is_some, result), rc, _) = Pool.Methods.AddDataDevs(
            self._pool_object, {"devices": _DEVICE_STRATEGY()}
        )

        num_devices_added = len(result)
        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy, {})

        if rc == StratisdErrors.OK:
            self.assertTrue(is_some)
            self.assertGreater(num_devices_added, 0)
        else:
            self.assertFalse(is_some)
            self.assertEqual(num_devices_added, 0)

        blockdev_object_paths = frozenset(result)

        # blockdevs exported on the D-Bus are those added and the existing data devs
        blockdevs2 = list(blockdevs(props={"Pool": pool}).search(managed_objects))
        blockdevs2_object_paths = frozenset([op for (op, _) in blockdevs2])
        self.assertEqual(
            blockdevs2_object_paths,
            blockdev_object_paths.union(frozenset(self._blockdev_paths)),
        )

        # no duplicates in the object paths
        self.assertEqual(len(blockdevs2) - len(self._blockdev_paths), num_devices_added)

        # There are no blockdevs but for those in this pool
        blockdevs3 = blockdevs(props={}).search(managed_objects)
        self.assertEqual(
            len(list(blockdevs3)), num_devices_added + len(self._data_devices)
        )

        # There are no cachedevs belonging to this pool
        blockdevs4 = blockdevs(props={"Pool": pool, "Tier": 1}).search(managed_objects)
        self.assertEqual(list(blockdevs4), [])
