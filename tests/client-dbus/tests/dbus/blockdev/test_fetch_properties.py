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
Test accessing properties of a blockdev using FetchProperties interface.
"""

# isort: LOCAL
from stratisd_client_dbus import FetchProperties, Manager, get_object
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase


class FetchPropertiesTestCase(SimTestCase):
    """
    Set up a pool with a name.
    """

    _POOLNAME = "fetchprops"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        proxy = get_object(TOP_OBJECT)
        ((_, (_, self._bd_object_paths)), _, _) = Manager.Methods.CreatePool(
            proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": ["/dev/one", "/dev/two", "/dev/red", "/dev/blue"],
            },
        )
        Manager.Methods.ConfigureSimulator(proxy, {"denominator": 8})

    def testFetchSizeProperty(self):
        """
        Test FetchProperties for blockdev property, TotalPhysicalSize
        """
        one_blockdev = get_object(self._bd_object_paths[0])

        (size_success, size) = FetchProperties.Methods.GetProperties(
            one_blockdev, {"properties": ["TotalPhysicalSize"]}
        )["TotalPhysicalSize"]

        self.assertEqual(size_success, True)
        self.assertTrue(size.isnumeric())
