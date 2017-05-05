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

import time
import unittest

from stratisd_client_dbus import MOPool
from stratisd_client_dbus import Manager
from stratisd_client_dbus import StratisdErrorsGen
from stratisd_client_dbus import ObjectManager
from stratisd_client_dbus import get_object
from stratisd_client_dbus import pools

from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import _device_list
from .._misc import Service

_DEVICE_STRATEGY = _device_list(0)


class Create2TestCase(unittest.TestCase):
    """
    Test 'create'.
    """
    _POOLNAME = 'deadpool'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        self._errors = StratisdErrorsGen.get_object()
        Manager.Methods.ConfigureSimulator(self._proxy, denominator=8)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testCreate(self):
        """
        Type of result should always be correct.

        If rc is OK, then pool must exist.
        """
        devs = _DEVICE_STRATEGY.example()
        ((poolpath, devnodes), rc, _) = Manager.Methods.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=(True, 0),
           force=False,
           devices=devs
        )

        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy)
        all_pools = [x for x in pools(managed_objects)]
        result = next(pools(managed_objects, {'Name': self._POOLNAME}), None)

        if rc == self._errors.OK:
            self.assertIsNotNone(result)
            (pool, table) = result
            self.assertEqual(pool, poolpath)
            self.assertEqual(len(all_pools), 1)
            self.assertLessEqual(len(devnodes), len(devs))

            pool_info = MOPool(table)
            self.assertLessEqual(
                int(pool_info.TotalPhysicalUsed()),
                int(pool_info.TotalPhysicalSize())
            )
        else:
            self.assertIsNone(result)
            self.assertEqual(len(all_pools), 0)

    def testCreateBadRAID(self):
        """
        Creation should always fail if RAID value is wrong.
        """
        redundancy_values = Manager.Properties.RedundancyValues.Get(self._proxy)

        devs = _DEVICE_STRATEGY.example()
        (_, rc, _) = Manager.Methods.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=(True, len(redundancy_values)),
           force=False,
           devices=devs
        )
        self.assertEqual(rc, self._errors.ERROR)

class Create3TestCase(unittest.TestCase):
    """
    Test 'create' on name collision.
    """
    _POOLNAME = 'deadpool'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        self._errors = StratisdErrorsGen.get_object()
        Manager.Methods.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=(True, 0),
           force=False,
           devices=_DEVICE_STRATEGY.example()
        )
        Manager.Methods.ConfigureSimulator(self._proxy, denominator=8)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testCreate(self):
        """
        Create should fail trying to create new pool with same name as previous.
        """
        pools1 = pools(ObjectManager.Methods.GetManagedObjects(self._proxy))

        (_, rc, _) = Manager.Methods.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=(True, 0),
           force=False,
           devices=_DEVICE_STRATEGY.example()
        )
        expected_rc = self._errors.ALREADY_EXISTS
        self.assertEqual(rc, expected_rc)

        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy)
        pools2 = [x for x in pools(managed_objects)]
        pool = next(pools(managed_objects, {'Name': self._POOLNAME}), None)

        self.assertIsNotNone(pool)
        self.assertEqual(
           frozenset(x for (x, y) in pools1),
           frozenset(x for (x, y) in pools2)
        )
