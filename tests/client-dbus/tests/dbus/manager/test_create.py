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

from stratisd_client_dbus import MOPool
from stratisd_client_dbus import Manager
from stratisd_client_dbus import StratisdErrors
from stratisd_client_dbus import ObjectManager
from stratisd_client_dbus import get_object
from stratisd_client_dbus import pools

from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase
from .._misc import device_name_list

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
        Manager.Methods.ConfigureSimulator(self._proxy, {"denominator": 8})

    def testCreate(self):
        """
        Type of result should always be correct.

        If rc is OK, then pool must exist.
        """
        devs = _DEVICE_STRATEGY()
        ((_, _, (poolpath, devnodes)), rc, _) = Manager.Methods.CreatePool(
            self._proxy,
            {"name": self._POOLNAME, "redundancy": (True, 0), "devices": devs},
        )

        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        all_pools = [x for x in pools().search(managed_objects)]
        result = next(
            pools(props={"Name": self._POOLNAME}).search(managed_objects), None
        )

        if rc == StratisdErrors.OK:
            self.assertIsNotNone(result)
            (pool, table) = result
            self.assertEqual(pool, poolpath)
            self.assertEqual(len(all_pools), 1)
            self.assertLessEqual(len(devnodes), len(devs))

            pool_info = MOPool(table)
            self.assertLessEqual(
                int(pool_info.TotalPhysicalUsed()), int(pool_info.TotalPhysicalSize())
            )
        else:
            self.assertIsNone(result)
            self.assertEqual(len(all_pools), 0)

    def testCreateBadRAID(self):
        """
        Creation should always fail if RAID value is wrong.
        """
        devs = _DEVICE_STRATEGY()
        (_, rc, _) = Manager.Methods.CreatePool(
            self._proxy,
            {"name": self._POOLNAME, "redundancy": (True, 1), "devices": devs},
        )
        self.assertEqual(rc, StratisdErrors.ERROR)


class Create3TestCase(SimTestCase):
    """
    Test 'create' on name collision.
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
        Manager.Methods.ConfigureSimulator(self._proxy, {"denominator": 8})

    def testCreate(self):
        """
        Create should fail trying to create new pool with same name as previous.
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
        expected_rc = StratisdErrors.ALREADY_EXISTS
        self.assertEqual(rc, expected_rc)

        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        pools2 = [x for x in pools().search(managed_objects)]
        pool = next(pools(props={"Name": self._POOLNAME}).search(managed_objects), None)

        self.assertIsNotNone(pool)
        self.assertEqual(
            frozenset(x for (x, y) in pools1), frozenset(x for (x, y) in pools2)
        )
