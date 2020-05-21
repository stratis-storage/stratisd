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
Test accessing properties of a pool using FetchProperties interface.
"""

# isort: LOCAL
from stratisd_client_dbus import FetchProperties, FetchPropertiesR1, Manager, get_object
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase, device_name_list

_DEVICE_STRATEGY = device_name_list(1)


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
        ((_, (pool_object_path, _)), _, _) = Manager.Methods.CreatePool(
            proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": _DEVICE_STRATEGY(),
            },
        )
        self._pool_object = get_object(pool_object_path)

    def test_fetch_size_property(self):
        """
        Test FetchProperties for pool property, TotalPhysicalSize
        """
        (size_success, size) = FetchProperties.Methods.GetProperties(
            self._pool_object, {"properties": ["TotalPhysicalSize"]}
        )["TotalPhysicalSize"]

        self.assertEqual(size_success, True)
        self.assertTrue(size.isnumeric())

        (size_success, size) = FetchPropertiesR1.Methods.GetProperties(
            self._pool_object, {"properties": ["TotalPhysicalSize"]}
        )["TotalPhysicalSize"]

        self.assertEqual(size_success, True)
        self.assertTrue(size.isnumeric())

    def test_fetch_used_size_property(self):
        """
        Test FetchProperties for pool property, TotalPhysicalUsed
        """
        (size_success, size) = FetchProperties.Methods.GetProperties(
            self._pool_object, {"properties": ["TotalPhysicalUsed"]}
        )["TotalPhysicalUsed"]

        self.assertEqual(size_success, True)
        self.assertTrue(size.isnumeric())

        (size_success, size) = FetchPropertiesR1.Methods.GetProperties(
            self._pool_object, {"properties": ["TotalPhysicalUsed"]}
        )["TotalPhysicalUsed"]

        self.assertEqual(size_success, True)
        self.assertTrue(size.isnumeric())

    def test_fetch_has_cache_property(self):
        """
        Test FetchProperties.r1 for pool HasCache property
        """
        (has_cache_success, has_cache) = FetchPropertiesR1.Methods.GetProperties(
            self._pool_object, {"properties": ["HasCache"]}
        )["HasCache"]

        self.assertEqual(has_cache_success, True)
        # dbus-python Booleans are actually ints, but they define equality
        # that works for bools
        self.assertIn(has_cache, (True, False))

    def test_fetch_data_crypt_key_description(self):
        """
        Test FetchProperties.r1 for pool KeyDescription
        """
        property_name = "KeyDescription"
        (key_desc_success, key_desc) = FetchPropertiesR1.Methods.GetProperties(
            self._pool_object, {"properties": [property_name]}
        )[property_name]

        self.assertEqual(key_desc_success, True)
        self.assertEqual(key_desc, (False, ""))
