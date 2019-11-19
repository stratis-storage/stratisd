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
Test destroying a filesystem in a pool.
"""

from stratisd_client_dbus import Manager
from stratisd_client_dbus import ObjectManager
from stratisd_client_dbus import Pool
from stratisd_client_dbus import StratisdErrors
from stratisd_client_dbus import filesystems
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase
from .._misc import device_name_list

_DEVICE_STRATEGY = device_name_list()


class DestroyFSTestCase(SimTestCase):
    """
    Test with an empty pool.
    """

    _POOLNAME = "deadpool"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        self._proxy = get_object(TOP_OBJECT)
        ((_, (poolpath, _)), _, _) = Manager.Methods.CreatePool(
            self._proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": _DEVICE_STRATEGY(),
            },
        )
        self._pool_object = get_object(poolpath)
        Manager.Methods.ConfigureSimulator(self._proxy, {"denominator": 8})

    def testDestroyNone(self):
        """
        Test calling with no actual volume specification. An empty volume
        list should always succeed, and it should not decrease the
        number of volumes.
        """
        ((_, result_changed), rc, _) = Pool.Methods.DestroyFilesystems(
            self._pool_object, {"filesystems": []}
        )

        self.assertEqual(len(result_changed), 0)
        self.assertEqual(rc, StratisdErrors.OK)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )
        self.assertEqual(len(list(result)), 0)

    def testDestroyOne(self):
        """
        Test calling with a non-existant object path. This should succeed,
        because at the end the filesystem is not there.
        """
        ((_, result), rc, _) = Pool.Methods.DestroyFilesystems(
            self._pool_object, {"filesystems": ["/"]}
        )
        self.assertEqual(len(result), 0)
        self.assertEqual(rc, StratisdErrors.OK)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )
        self.assertEqual(len(list(result)), 0)


class DestroyFSTestCase1(SimTestCase):
    """
    Make a filesystem for the pool.
    """

    _POOLNAME = "deadpool"
    _FSNAME = "thunk"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        self._proxy = get_object(TOP_OBJECT)
        ((_, (poolpath, _)), _, _) = Manager.Methods.CreatePool(
            self._proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": _DEVICE_STRATEGY(),
            },
        )
        self._pool_object = get_object(poolpath)
        ((_, self._filesystems), _, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {"specs": [(self._FSNAME, "", None)]}
        )
        Manager.Methods.ConfigureSimulator(self._proxy, {"denominator": 8})

    def testDestroyOne(self):
        """
        Test calling by specifying the object path. Assume that destruction
        should always succeed.
        """
        fs_object_path = self._filesystems[0][0]
        ((is_some, result), rc, _) = Pool.Methods.DestroyFilesystems(
            self._pool_object, {"filesystems": [fs_object_path]}
        )

        self.assertTrue(is_some)
        self.assertEqual(len(result), 1)
        self.assertEqual(rc, StratisdErrors.OK)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )
        self.assertEqual(len(list(result)), 0)

    def testDestroyTwo(self):
        """
        Test calling by specifying one existing volume name and one
        non-existing. Should succeed, but only the existing name should be
        returned.
        """
        fs_object_path = self._filesystems[0][0]
        ((is_some, result), rc, _) = Pool.Methods.DestroyFilesystems(
            self._pool_object, {"filesystems": [fs_object_path, "/"]}
        )

        self.assertTrue(is_some)
        self.assertEqual(len(result), 1)
        self.assertEqual(rc, StratisdErrors.OK)

        result = filesystems().search(
            ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        )
        self.assertEqual(len(list(result)), 0)
