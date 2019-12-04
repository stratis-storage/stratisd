# Copyright 2019 Red Hat, Inc.
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
Test accessing properties of a filesystem using FetchProperties interface.
"""

# isort: LOCAL
from stratisd_client_dbus import FetchProperties, Manager, Pool, get_object
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase, device_name_list

_DEVICE_STRATEGY = device_name_list()


class FetchPropertiesTestCase(SimTestCase):
    """
    Set up a pool with a name and a filesystem.
    """

    _POOLNAME = "fetchprops"
    _FSNAME = "fs"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        proxy = get_object(TOP_OBJECT)
        ((_, (pool_object_path, _)), _, _) = Manager.Methods.CreatePool(
            proxy,
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
        Manager.Methods.ConfigureSimulator(proxy, {"denominator": 8})

    def testFetchUsedProperty(self):
        """
        Test FetchProperties for filesystem property, Used
        """
        filesystem = get_object(self._filesystem_object_path)

        (used_success, used) = FetchProperties.Methods.GetProperties(
            filesystem, {"properties": ["Used"]}
        )["Used"]

        self.assertEqual(used_success, True)
        self.assertTrue(used.isnumeric())
