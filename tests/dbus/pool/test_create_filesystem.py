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
Test creating a filesystem in a pool.
"""

import time
import unittest


from stratisd_client_dbus import Manager
from stratisd_client_dbus import Pool
from stratisd_client_dbus import StratisdErrorsGen
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from stratisd_client_dbus._implementation import PoolSpec

from .._constants import _DEVICES

from .._misc import checked_call
from .._misc import _device_list
from .._misc import Service

_PN = PoolSpec.MethodNames

class CreateFSTestCase(unittest.TestCase):
    """
    Test with an empty pool.
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
        self._devs = [d.device_node for d in _device_list(_DEVICES, 1)]
        (result, _, _) = Manager.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=0,
           force=False,
           devices=self._devs
        )
        self._pool_object = get_object(result)
        Manager.ConfigureSimulator(self._proxy, denominator=8)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testCreate(self):
        """
        Test calling with no actual volume specification. An empty volume
        list should always succeed, and it should not increase the
        number of volumes.
        """
        (result, rc, _) = checked_call(
           Pool.CreateFilesystems(self._pool_object, specs=[]),
           PoolSpec.OUTPUT_SIGS[_PN.CreateFilesystems]
        )

        self.assertEqual(len(result), 0)
        self.assertEqual(rc, self._errors.OK)

        (result, rc, _) = checked_call(
           Pool.ListFilesystems(self._pool_object),
           PoolSpec.OUTPUT_SIGS[_PN.ListFilesystems]
        )
        self.assertEqual(rc, self._errors.OK)
        self.assertEqual(len(result), 0)


class CreateFSTestCase1(unittest.TestCase):
    """
    Make a filesystem for the pool.
    """

    _POOLNAME = 'deadpool'
    _VOLNAME = 'thunk'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        self._errors = StratisdErrorsGen.get_object()
        self._devs = [d.device_node for d in _device_list(_DEVICES, 1)]
        (result, _, _) = Manager.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=0,
           force=False,
           devices=self._devs
        )
        self._pool_object = get_object(result)
        Pool.CreateFilesystems(
           self._pool_object,
           specs=[(self._VOLNAME, '', None)]
        )
        Manager.ConfigureSimulator(self._proxy, denominator=8)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testCreate(self):
        """
        Test calling by specifying a volume name. Because there is already
        a volume with the given name, the creation of the new volume should
        fail, and no additional volume should be created.
        """
        (result, rc, _) = checked_call(
           Pool.CreateFilesystems(
              self._pool_object,
              specs=[(self._VOLNAME, "", None)]
           ),
           PoolSpec.OUTPUT_SIGS[_PN.CreateFilesystems]
        )

        self.assertEqual(rc, self._errors.LIST_FAILURE)
        self.assertEqual(len(result), 1)

        (result, rc, _) = result[0]

        self.assertEqual(rc, self._errors.ALREADY_EXISTS)

        (result, rc, _) = checked_call(
           Pool.ListFilesystems(self._pool_object),
           PoolSpec.OUTPUT_SIGS[_PN.ListFilesystems]
        )
        self.assertEqual(rc, self._errors.OK)
        self.assertEqual(len(result), 1)
