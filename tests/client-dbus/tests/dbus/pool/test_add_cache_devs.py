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
from stratisd_client_dbus._stratisd_constants import BlockDevTiers

from .._misc import SimTestCase, device_name_list

_DEVICE_STRATEGY = device_name_list(1)


class AddCacheDevsTestCase1(SimTestCase):
    """
    Test adding cachedevs to a pool which is initially empty.
    """

    _POOLNAME = "deadpool"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        self._proxy = get_object(TOP_OBJECT)
        self._data_devices = _DEVICE_STRATEGY()
        ((_, (poolpath, _)), _, _) = Manager.Methods.CreatePool(
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
        Adding an empty list of cache devs should fail.
        """
        ((is_some, _), rc, _) = Pool.Methods.AddCacheDevs(
            self._pool_object, {"devices": []}
        )

        self.assertFalse(is_some)
        self.assertEqual(rc, StratisdErrors.ERROR)

    def testSomeDevs(self):
        """
        Adding a non-empty list of cache devs should succeed.
        """
        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (pool, _) = next(pools(props={"Name": self._POOLNAME}).search(managed_objects))

        ((is_some, result), rc, _) = Pool.Methods.AddCacheDevs(
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

        blockdev_paths = frozenset(result)

        # blockdevs exported on the D-Bus are exactly those added
        blockdevs2 = list(
            blockdevs(props={"Pool": pool, "Tier": BlockDevTiers.Cache}).search(
                managed_objects
            )
        )
        blockdevs2_paths = frozenset([op for (op, _) in blockdevs2])
        self.assertEqual(blockdevs2_paths, blockdev_paths)

        # no duplicates in the object paths
        self.assertEqual(len(blockdevs2), num_devices_added)

        # There are no cache blockdevs but for those in this pool
        blockdevs3 = blockdevs(props={"Tier": BlockDevTiers.Cache}).search(
            managed_objects
        )
        self.assertEqual(len(list(blockdevs3)), num_devices_added)

        # There must be datadevs belonging to this pool as it was created
        blockdevs4 = blockdevs(props={"Pool": pool, "Tier": 0}).search(managed_objects)
        self.assertEqual(len(list(blockdevs4)), len(self._data_devices))
