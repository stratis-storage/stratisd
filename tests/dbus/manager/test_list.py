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
Test 'list'.
"""

import time
import unittest

from stratisd_client_dbus import StratisdErrorsGen
from stratisd_client_dbus import Manager
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from stratisd_client_dbus._implementation import ManagerSpec

from .._constants import _DEVICES

from .._misc import checked_call
from .._misc import _device_list
from .._misc import Service

_MN = ManagerSpec.MethodNames

class ListTestCase(unittest.TestCase):
    """
    Test 'list'.
    """

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

    def testList(self):
        """
        List should just succeed.
        """
        (result, rc, _) = checked_call(
           Manager.ListPools(self._proxy),
           ManagerSpec.OUTPUT_SIGS[_MN.ListPools]
        )

        self.assertEqual(result, [])
        self.assertEqual(rc, self._errors.OK)


class List2TestCase(unittest.TestCase):
    """
    Test 'list' with something actually to list.
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
           devices=[d.device_node for d in _device_list(_DEVICES, 1)]
        )
        Manager.ConfigureSimulator(self._proxy, denominator=8)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testList(self):
        """
        List should just succeed.
        """
        (result, rc, _) = checked_call(
           Manager.ListPools(self._proxy),
           ManagerSpec.OUTPUT_SIGS[_MN.ListPools]
        )

        self.assertEqual(rc, self._errors.OK)
        self.assertEqual(len(result), 1)
