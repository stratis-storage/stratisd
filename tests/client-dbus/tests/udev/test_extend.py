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
Test extending an encrypted pool.
"""

# isort: STDLIB
import json
from time import sleep

# isort: LOCAL
from stratisd_client_dbus import Manager, Pool, get_object
from stratisd_client_dbus._constants import TOP_OBJECT

from ._utils import OptionalKeyServiceContextManager, UdevTest, create_pool, get_pools


class TestExtendOnAddData(UdevTest):
    """
    Test creating an encrypted pool, unsetting the key, and adding a new data device.

    This is a regression test for a bug.
    """

    def test_failed_extend_regression(self):
        """
        * Create encrypted pool
        * Unset key
        * Data data device
        * Test whether allocated size is larger
        """

        device_tokens = self._lb_mgr.create_devices(3)
        devnodes = self._lb_mgr.device_files(device_tokens)

        with OptionalKeyServiceContextManager(
            key_spec=[("testkey", "testkey")]
        ) as key_descriptions:
            key_description = key_descriptions[0]

            self.wait_for_pools(0)
            (_, (pool_object_path, _)) = create_pool(
                "testpool",
                devnodes[:2],
                key_description=([(key_description, None)]),
            )
            self.wait_for_pools(1)

            (metadata, _, _) = Pool.Methods.Metadata(
                get_object(pool_object_path), {"current": False}
            )
            metadata_dct = json.loads(metadata)
            allocs = metadata_dct["backstore"]["cap"]["allocs"]

        with OptionalKeyServiceContextManager(key_spec=[("testkey", "testkey")]):
            (_, rc, _) = Manager.Methods.UnsetKey(
                get_object(TOP_OBJECT), {"key_desc": "testkey"}
            )
            self.assertEqual(0, rc)

            (pool_object_path, _) = get_pools(name="testpool")[0]
            (_, rc, _) = Pool.Methods.AddDataDevs(
                get_object(pool_object_path), {"devices": [devnodes[2]]}
            )
            self.assertEqual(rc, 0)

            sleep(10)

            (metadata, _, _) = Pool.Methods.Metadata(
                get_object(pool_object_path), {"current": False}
            )
            metadata_dct = json.loads(metadata)
            new_allocs = metadata_dct["backstore"]["cap"]["allocs"]
            self.assertNotEqual(allocs, new_allocs)
