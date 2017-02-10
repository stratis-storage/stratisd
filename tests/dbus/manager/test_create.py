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

from stratisd_client_dbus import Manager
from stratisd_client_dbus import StratisdErrorsGen
from stratisd_client_dbus import get_managed_objects
from stratisd_client_dbus import get_object

from stratisd_client_dbus._implementation import ManagerSpec
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import checked_call
from .._misc import _device_list
from .._misc import Service

_MN = ManagerSpec.MethodNames
_MP = ManagerSpec.PropertyNames

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
        Manager.ConfigureSimulator(self._proxy, denominator=8)

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
        ((poolpath, devnodes), rc, _) = checked_call(
           Manager.CreatePool(
              self._proxy,
              name=self._POOLNAME,
              redundancy=0,
              force=False,
              devices=devs
           ),
           ManagerSpec.OUTPUT_SIGS[_MN.CreatePool]
        )

        managed_objects = get_managed_objects(self._proxy)
        pools = [x for x in managed_objects.pools()]
        result = managed_objects.get_pool_by_name(self._POOLNAME)

        if rc == self._errors.OK:
            self.assertIsNotNone(result)
            (pool, _) = result
            self.assertEqual(pool, poolpath)
            self.assertEqual(len(pools), 1)
            self.assertLessEqual(len(devnodes), len(devs))
        else:
            self.assertIsNone(result)
            self.assertEqual(len(pools), 0)

    def testCreateBadRAID(self):
        """
        Creation should always fail if RAID value is wrong.
        """
        redundancy_values = Manager.Properties.RedundancyValues(self._proxy)

        devs = _DEVICE_STRATEGY.example()
        (_, rc, _) = checked_call(
           Manager.CreatePool(
              self._proxy,
              name=self._POOLNAME,
              redundancy=len(redundancy_values),
              force=False,
              devices=devs
           ),
           ManagerSpec.OUTPUT_SIGS[_MN.CreatePool]
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
        Manager.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=0,
           force=False,
           devices=_DEVICE_STRATEGY.example()
        )
        Manager.ConfigureSimulator(self._proxy, denominator=8)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testCreate(self):
        """
        Create should fail trying to create new pool with same name as previous.
        """
        pools1 = get_managed_objects(self._proxy).pools()

        (_, rc, _) = checked_call(
           Manager.CreatePool(
              self._proxy,
              name=self._POOLNAME,
              redundancy=0,
              force=False,
              devices=_DEVICE_STRATEGY.example()
           ),
           ManagerSpec.OUTPUT_SIGS[_MN.CreatePool]
        )
        expected_rc = self._errors.ALREADY_EXISTS
        self.assertEqual(rc, expected_rc)

        managed_objects = get_managed_objects(self._proxy)
        pools2 = [x for x in managed_objects.pools()]
        pool = managed_objects.get_pool_by_name(self._POOLNAME)

        self.assertIsNotNone(pool)
        self.assertEqual(
           frozenset(x for (x, y) in pools1),
           frozenset(x for (x, y) in pools2)
        )
