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
Test DestroyPool.
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


class Destroy1TestCase(unittest.TestCase):
    """
    Test 'destroy' on empty database.

    'destroy' should always succeed on an empty database.
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

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testExecution(self):
        """
        Destroy should succeed.
        """
        (rc, message) = Manager.callMethod(
           self._proxy,
           "DestroyPool",
           self._POOLNAME
        )
        self.assertEqual(rc, StratisdErrorsGen.get_object().OK)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)

        (_, rc1, _) = Manager.callMethod(
           self._proxy,
           "GetPoolObjectPath",
           self._POOLNAME
        )

        expected_rc = StratisdErrorsGen.get_object().POOL_NOTFOUND
        self.assertEqual(rc1, expected_rc)


class Destroy2TestCase(unittest.TestCase):
    """
    Test 'destroy' on database which contains the given pool.
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
           "CreatePool",
           self._POOLNAME,
           0,
           [d.device_node for d in _device_list(_DEVICES, 1)]
        )

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testExecution(self):
        """
        The pool was just created, so must be destroyable.
        """
        (rc, message) = Manager.callMethod(
           self._proxy,
           "DestroyPool",
           self._POOLNAME
        )
        self.assertEqual(rc, StratisdErrorsGen.get_object().OK)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)

        (_, rc1, _) = Manager.callMethod(
           self._proxy,
           "GetPoolObjectPath",
           self._POOLNAME
        )

        expected_rc = StratisdErrorsGen.get_object().POOL_NOTFOUND
        self.assertEqual(rc1, expected_rc)


class Destroy3TestCase(unittest.TestCase):
    """
    Test 'destroy' on database which contains the given pool and a volume.
    """
    _POOLNAME = 'deadpool'
    _VOLNAME = 'vol'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        Create a pool and a filesystem.
        """
        self._service = Service()
        self._service.setUp()

        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        (poolpath, _, _) = Manager.callMethod(
           self._proxy,
           "CreatePool",
           self._POOLNAME,
           0,
           [d.device_node for d in _device_list(_DEVICES, 1)]
        )
        (_, _, _) = Pool.callMethod(
           get_object(poolpath),
           "CreateFilesystems",
           [(self._VOLNAME, '', 0)]
        )

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    @unittest.expectedFailure
    def testExecution(self):
        """
        This should fail since it has a filesystem on it.
        """
        (rc, message) = Manager.callMethod(
           self._proxy,
           "DestroyPool",
           self._POOLNAME
        )
        self.assertNotEqual(rc, StratisdErrorsGen.get_object().OK)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)
