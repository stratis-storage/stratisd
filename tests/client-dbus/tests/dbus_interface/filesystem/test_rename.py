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
Test renaming a filesystem.
"""

# isort: LOCAL
from stratisd_client_dbus import (
    Filesystem,
    Manager,
    ObjectManager,
    Pool,
    StratisdErrors,
    filesystems,
    get_object,
)
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase, device_name_list

_DEVICE_STRATEGY = device_name_list(1)


class SetNameTestCase(SimTestCase):
    """
    Set up a pool with a name and one filesystem.
    """

    _POOLNAME = "deadpool"
    _FSNAME = "fs"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        self._proxy = get_object(TOP_OBJECT)
        ((_, (pool_object_path, _)), _, _) = Manager.Methods.CreatePool(
            self._proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": _DEVICE_STRATEGY(),
            },
        )
        pool_object = get_object(pool_object_path)
        ((_, created), _, _) = Pool.Methods.CreateFilesystems(
            pool_object, {"specs": [self._FSNAME]}
        )
        self._filesystem_object_path = created[0][0]

    def test_null_mapping(self):
        """
        Test rename to same name.
        """
        filesystem = get_object(self._filesystem_object_path)
        ((is_some, result), return_code, _) = Filesystem.Methods.SetName(
            filesystem, {"name": self._FSNAME}
        )

        self.assertEqual(return_code, StratisdErrors.OK)
        self.assertFalse(is_some)
        self.assertEqual(result, "0" * 32)

    def test_new_name(self):
        """
        Test rename to new name.
        """
        filesystem = get_object(self._filesystem_object_path)
        (result, return_code, _) = Filesystem.Methods.SetName(
            filesystem, {"name": "new"}
        )

        self.assertEqual(return_code, StratisdErrors.OK)
        self.assertTrue(result)

        managed_objects = ObjectManager.Methods.GetManagedObjects(self._proxy, {})
        (fs_object_path, _) = next(
            filesystems(props={"Name": "new"}).search(managed_objects)
        )
        self.assertEqual(self._filesystem_object_path, fs_object_path)

        fs_object_path = next(
            filesystems(props={"Name": self._FSNAME}).search(managed_objects), None
        )
        self.assertIsNone(fs_object_path)
