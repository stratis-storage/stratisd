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
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from .._constants import _DEVICES

from .._misc import _device_list
from .._misc import Service

_MN = Manager.MethodNames

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
        (_, _) = Manager.callMethod(self._proxy, _MN.ConfigureSimulator, 8)

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
        (result, rc, message) = Manager.callMethod(
           self._proxy,
           _MN.CreatePool,
           self._POOLNAME,
           0,
           [d.device_node for d in _device_list(_DEVICES, 1)]
        )
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)

        (pool, rc1, _) = Manager.callMethod(
           self._proxy,
           _MN.GetPoolObjectPath,
           self._POOLNAME
        )

        ok = StratisdErrorsGen.get_object().OK
        if rc == ok:
            self.assertEqual(pool, result)
            self.assertEqual(rc1, ok)
        else:
            expected = StratisdErrorsGen.get_object().POOL_NOTFOUND
            self.assertEqual(rc1, expected)


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
        Manager.callMethod(
           self._proxy,
           _MN.CreatePool,
           self._POOLNAME,
           0,
           [d.device_node for d in _device_list(_DEVICES, 1)]
        )
        (_, _) = Manager.callMethod(self._proxy, _MN.ConfigureSimulator, 8)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testCreate(self):
        """
        Create should fail trying to create new pool with same name as previous.
        """
        (result, rc, message) = Manager.callMethod(
           self._proxy,
           _MN.CreatePool,
           self._POOLNAME,
           0,
           [d.device_node for d in _device_list(_DEVICES, 1)]
        )
        expected_rc = StratisdErrorsGen.get_object().ALREADY_EXISTS
        self.assertEqual(rc, expected_rc)
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)

        (_, rc1, _) = Manager.callMethod(
           self._proxy,
           _MN.GetPoolObjectPath,
           self._POOLNAME
        )

        self.assertEqual(rc1, StratisdErrorsGen.get_object().OK)
