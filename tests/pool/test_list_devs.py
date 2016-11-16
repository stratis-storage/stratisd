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
Test 'list' of blockdevs in pool.
"""

import time
import unittest

from stratisd_client_dbus import Manager
from stratisd_client_dbus import Pool
from stratisd_client_dbus import StratisdErrorsGen
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from .._constants import _DEVICES

from .._misc import _device_list
from .._misc import Service

_MN = Manager.MethodNames
_PN = Pool.MethodNames


class ListTestCase(unittest.TestCase):
    """
    Test listing devices for a pool.
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
        self._devs = [d.device_node for d in _device_list(_DEVICES, 1)]
        (result, _, _) = Manager.callMethod(
           self._proxy,
           _MN.CreatePool,
           self._POOLNAME,
           0,
           self._devs
        )
        self._pool_object = get_object(result)
        (_, _) = Manager.callMethod(self._proxy, _MN.ConfigureSimulator, 8)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testList(self):
        """
        List should succeed and contain the same number of entries as
        devices in pool.
        """
        (result, rc, message) = Pool.callMethod(self._pool_object, _PN.ListDevs)
        self.assertIsInstance(result, list)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)

        self.assertEqual(len(result), len(self._devs))
        self.assertEqual(rc, StratisdErrorsGen.get_object().OK)
