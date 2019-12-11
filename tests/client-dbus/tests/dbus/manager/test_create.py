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
Test 'CreatePool'.
"""

# isort: LOCAL
from stratisd_client_dbus import (
    Manager,
    ObjectManager,
    StratisdErrors,
    get_object,
    pools,
)
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase, device_name_list

_DEVICE_STRATEGY = device_name_list()


class Create2TestCase(SimTestCase):
    """
    Test 'create'.
    """

    _POOLNAME = "deadpool"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        self._proxy = get_object(TOP_OBJECT)
        self._devs = _DEVICE_STRATEGY()
        Manager.Methods.ConfigureSimulator(self._proxy, {"denominator": 8})

    def testCreate(self):
        """
        Type of result should always be correct.

        If rc is OK, then pool must exist.
        """
        ((_, (poolpath, devnodes)), rc, _) = Manager.Methods.CreatePool(
            self._proxy,
            {"name": self._POOLNAME, "redundancy": (True, 0), "devices": self._devs},
        )

        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        all_pools = list(pools().search(managed_objects))
        result = next(
            pools(props={"Name": self._POOLNAME}).search(managed_objects), None
        )

        if rc == StratisdErrors.OK:
            self.assertIsNotNone(result)
            (pool, _) = result
            self.assertEqual(pool, poolpath)
            self.assertEqual(len(all_pools), 1)
            self.assertLessEqual(len(devnodes), len(self._devs))
        else:
            self.assertIsNone(result)
            self.assertEqual(len(all_pools), 0)

    def testCreateBadRAID(self):
        """
        Creation should always fail if RAID value is wrong.
        """
        (_, rc, _) = Manager.Methods.CreatePool(
            self._proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 1),
                "devices": _DEVICE_STRATEGY(),
            },
        )
        self.assertEqual(rc, StratisdErrors.ERROR)


class Create3TestCase(SimTestCase):
    """
    Test 'create' on name collision and different arguments.
    """

    _POOLNAME = "deadpool"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        self._proxy = get_object(TOP_OBJECT)
        Manager.Methods.CreatePool(
            self._proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": _DEVICE_STRATEGY(),
            },
        )

    def testCreateDifferentBlockdevs(self):
        """
        Create should fail trying to create new pool with same name
        and different blockdevs from previous.
        """
        pools1 = pools().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )

        (_, rc, _) = Manager.Methods.CreatePool(
            self._proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": _DEVICE_STRATEGY(),
            },
        )
        self.assertEqual(rc, StratisdErrors.ERROR)

        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        pools2 = list(pools().search(managed_objects))
        pool = next(pools(props={"Name": self._POOLNAME}).search(managed_objects), None)

        self.assertIsNotNone(pool)
        self.assertEqual(
            frozenset(x for (x, y) in pools1), frozenset(x for (x, y) in pools2)
        )


class Create4TestCase(SimTestCase):
    """
    Test 'create' when passing same arguments twice (idempotence).
    """

    _POOLNAME = "idempool"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        self._proxy = get_object(TOP_OBJECT)
        self._blockdevs = ["/dev/one", "/dev/two", "/dev/red", "/dev/blue"]
        Manager.Methods.CreatePool(
            self._proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": self._blockdevs,
            },
        )

    def testCreateSameBlockdevs(self):
        """
        Create should succeed trying to create new pool with same name
        and same blockdevs as previous.
        """
        pools1 = pools().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )

        ((is_some, _), rc, _) = Manager.Methods.CreatePool(
            self._proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": self._blockdevs,
            },
        )
        self.assertEqual(rc, StratisdErrors.OK)
        self.assertFalse(is_some)

        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        pools2 = list(pools().search(managed_objects))
        pool = next(pools(props={"Name": self._POOLNAME}).search(managed_objects), None)

        self.assertIsNotNone(pool)
        self.assertEqual(
            frozenset(x for (x, y) in pools1), frozenset(x for (x, y) in pools2)
        )
