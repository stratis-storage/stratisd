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

import time
import unittest

from stratisd_client_dbus import Manager
from stratisd_client_dbus import Pool
from stratisd_client_dbus import StratisdErrors
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import _device_list
from .._misc import Service

_DEVICE_STRATEGY = _device_list(1)


class AddDevsTestCase(unittest.TestCase):
    """
    Test adding devices to a pool which is initially empty.
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
        ((poolpath, _), _, _) = Manager.Methods.CreatePool(
           self._proxy,
           {
              'name': self._POOLNAME,
              'redundancy': (True, 0),
              'force': False,
              'devices': []
           }
        )
        self._pool_object = get_object(poolpath)
        Manager.Methods.ConfigureSimulator(self._proxy, {'denominator': 8})

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testEmptyDevs(self):
        """
        Adding an empty list of devs should leave the pool empty.
        """
        (result, rc, _) = Pool.Methods.AddDevs(
           self._pool_object,
           {
              'force': False,
              'devices': []
           }
        )

        self.assertEqual(len(result), 0)
        self.assertEqual(rc, StratisdErrors.OK)

    def testSomeDevs(self):
        """
        Adding a non-empty list of devs should increase the number of devs
        in the pool.
        """
        (result, rc, _) = Pool.Methods.AddDevs(
           self._pool_object,
           {
              'force': False,
              'devices': _DEVICE_STRATEGY.example()
           }
        )

        num_devices_added = len(result)
        if rc == StratisdErrors.OK:
            self.assertGreater(num_devices_added, 0)
        else:
            self.assertEqual(num_devices_added, 0)
