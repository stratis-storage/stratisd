# Copyright 2021 Red Hat, Inc.
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
Test binding and unbinding in sequence with other actions.
"""

# isort: STDLIB
import json
import os

# isort: LOCAL
from stratisd_client_dbus import Pool, StratisdErrors, get_object

from ._utils import (
    OptionalKeyServiceContextManager,
    ServiceContextManager,
    UdevTest,
    create_pool,
    random_string,
)


class TestBindingAndAddingTrustedUrl(UdevTest):
    """
    Test binding to a tang server with a trusted URL and subsequently
    adding data devices in various orders.
    """

    _TANG_URL = os.getenv("TANG_URL")
    _CLEVIS_CONFIG = {"url": _TANG_URL, "stratis:tang:trust_url": True}
    _CLEVIS_CONFIG_STR = json.dumps(_CLEVIS_CONFIG)

    def test_binding_and_adding(self):
        """
        Verify that binding and adding succeeds when url is trusted.
        """
        device_tokens = self._lb_mgr.create_devices(2)
        initial_devnodes = self._lb_mgr.device_files(device_tokens)

        device_tokens = self._lb_mgr.create_devices(2)
        added_devnodes = self._lb_mgr.device_files(device_tokens)

        (key_description, key) = ("key_spec", "data")
        with OptionalKeyServiceContextManager(key_spec=[(key_description, key)]):
            pool_name = random_string(5)
            (_, (pool_object_path, _)) = create_pool(
                pool_name, initial_devnodes, key_description=key_description
            )
            self.wait_for_pools(1)

            (_, exit_code, message) = Pool.Methods.Bind(
                get_object(pool_object_path),
                {"pin": "tang", "json": self._CLEVIS_CONFIG_STR},
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

            (_, exit_code, message) = Pool.Methods.AddDataDevs(
                get_object(pool_object_path), {"devices": added_devnodes}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

    def test_binding_unbinding_adding(self):
        """
        Test that binding, unbinding, and then adding devices works.
        """
        device_tokens = self._lb_mgr.create_devices(2)
        initial_devnodes = self._lb_mgr.device_files(device_tokens)

        device_tokens = self._lb_mgr.create_devices(2)
        added_devnodes = self._lb_mgr.device_files(device_tokens)

        (key_description, key) = ("key_spec", "data")
        with OptionalKeyServiceContextManager(key_spec=[(key_description, key)]):
            pool_name = random_string(5)
            (_, (pool_object_path, _)) = create_pool(
                pool_name, initial_devnodes, key_description=key_description
            )
            self.wait_for_pools(1)

            (_, exit_code, message) = Pool.Methods.Bind(
                get_object(pool_object_path),
                {"pin": "tang", "json": self._CLEVIS_CONFIG_STR},
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

            (_, exit_code, message) = Pool.Methods.Unbind(
                get_object(pool_object_path), {}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

            (_, exit_code, message) = Pool.Methods.AddDataDevs(
                get_object(pool_object_path), {"devices": added_devnodes}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

    def test_swap_binding(self):
        """
        Test that binding with clevis, unbinding with keyring, and then
        adding devices works.
        """
        device_tokens = self._lb_mgr.create_devices(2)
        initial_devnodes = self._lb_mgr.device_files(device_tokens)

        device_tokens = self._lb_mgr.create_devices(2)
        added_devnodes = self._lb_mgr.device_files(device_tokens)

        (key_description, key) = ("key_spec", "data")
        with OptionalKeyServiceContextManager(key_spec=[(key_description, key)]):
            pool_name = random_string(5)
            (_, (pool_object_path, _)) = create_pool(
                pool_name, initial_devnodes, key_description=key_description
            )
            self.wait_for_pools(1)

            (_, exit_code, message) = Pool.Methods.Bind(
                get_object(pool_object_path),
                {"pin": "tang", "json": self._CLEVIS_CONFIG_STR},
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

            (_, exit_code, message) = Pool.Methods.UnbindKeyring(
                get_object(pool_object_path), {}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

            (_, exit_code, message) = Pool.Methods.AddDataDevs(
                get_object(pool_object_path), {"devices": added_devnodes}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

    def test_swap_binding_2(self):
        """
        Test that binding with keyring, unbinding with clevis, and then
        adding devices works.
        """
        device_tokens = self._lb_mgr.create_devices(2)
        initial_devnodes = self._lb_mgr.device_files(device_tokens)

        device_tokens = self._lb_mgr.create_devices(2)
        added_devnodes = self._lb_mgr.device_files(device_tokens)

        (key_description, key) = ("key_spec", "data")
        with OptionalKeyServiceContextManager(key_spec=[(key_description, key)]):
            clevis_info = ("tang", self._CLEVIS_CONFIG_STR)

            pool_name = random_string(5)
            (_, (pool_object_path, _)) = create_pool(
                pool_name, initial_devnodes, clevis_info=clevis_info
            )
            self.wait_for_pools(1)

            (_, exit_code, message) = Pool.Methods.BindKeyring(
                get_object(pool_object_path), {"key_desc": key_description}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

            (_, exit_code, message) = Pool.Methods.Unbind(
                get_object(pool_object_path), {}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

            (_, exit_code, message) = Pool.Methods.AddDataDevs(
                get_object(pool_object_path), {"devices": added_devnodes}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

    def test_rebind_with_clevis(self):
        """
        Test that binding with clevis on creation, then rebinding, and then
        adding data devices works.
        """
        device_tokens = self._lb_mgr.create_devices(2)
        initial_devnodes = self._lb_mgr.device_files(device_tokens)

        device_tokens = self._lb_mgr.create_devices(2)
        added_devnodes = self._lb_mgr.device_files(device_tokens)

        with ServiceContextManager():
            pool_name = random_string(5)

            clevis_info = ("tang", self._CLEVIS_CONFIG_STR)

            (_, (pool_object_path, _)) = create_pool(
                pool_name, initial_devnodes, clevis_info=clevis_info
            )
            self.wait_for_pools(1)

            (_, exit_code, message) = Pool.Methods.RebindClevis(
                get_object(pool_object_path), {}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

            (_, exit_code, message) = Pool.Methods.AddDataDevs(
                get_object(pool_object_path), {"devices": added_devnodes}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)


class TestBindingAndRebindingKernelKeyring(UdevTest):
    """
    Test binding and rebinding with a key in the kernel keyring and adding
    data devices.
    """

    def test_rebind_with_new_key_description(self):
        """
        Test that binding with key on creation, rebinding with different key,
        and then adding data devices works.
        """
        device_tokens = self._lb_mgr.create_devices(2)
        initial_devnodes = self._lb_mgr.device_files(device_tokens)

        device_tokens = self._lb_mgr.create_devices(2)
        added_devnodes = self._lb_mgr.device_files(device_tokens)

        keys = [("key_desc_1", "key_data_1"), ("key_desc_2", "key_data_2")]
        with OptionalKeyServiceContextManager(key_spec=keys):
            pool_name = random_string(5)
            (_, (pool_object_path, _)) = create_pool(
                pool_name, initial_devnodes, key_description=keys[0][0]
            )
            self.wait_for_pools(1)

            (_, exit_code, message) = Pool.Methods.RebindKeyring(
                get_object(pool_object_path), {"key_desc": keys[1][0]}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

            (_, exit_code, message) = Pool.Methods.AddDataDevs(
                get_object(pool_object_path), {"devices": added_devnodes}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)
