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
Test binding and unbinding along with other actions.
"""

# isort: STDLIB
import json
import os

# isort: LOCAL
from stratisd_client_dbus import Pool, StratisdErrors, get_object

from ._utils import (
    OptionalKeyServiceContextManager,
    UdevTest,
    create_pool,
    random_string,
)

_TANG_URL = os.getenv("TANG_URL")


class TestBindingAndAdding(UdevTest):
    """
    Test binding followed by adding of data devices.
    """

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

            clevis_config = {"url": _TANG_URL, "stratis:tang:trust_url": True}

            (_, exit_code, message) = Pool.Methods.Bind(
                get_object(pool_object_path),
                {"pin": "tang", "json": json.dumps(clevis_config)},
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)

            (_, exit_code, message) = Pool.Methods.AddDataDevs(
                get_object(pool_object_path), {"devices": added_devnodes}
            )

            self.assertEqual(exit_code, StratisdErrors.OK, message)
