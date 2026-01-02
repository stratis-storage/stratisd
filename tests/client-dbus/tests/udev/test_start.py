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
Test starting additional stratisd processes.
"""

# isort: STDLIB
import os

# isort: LOCAL
from stratisd_client_dbus import Manager, Pool, StratisdErrors, get_object
from stratisd_client_dbus._constants import TOP_OBJECT

from ._utils import OptionalKeyServiceContextManager, UdevTest, create_pool


class TestFailedStart(UdevTest):
    """
    Test creating an encrypted pool and stopping it, removing the key from the keyring,
    and then start it.

    This is a regression test for a bug.
    """

    def test_failed_start_regression(self):
        """
        * Create encryption pool
        * Add cache
        * Stop pool
        * Unset key
        * Start pool (should fail)
        * Set key again
        * Start pool (should succeed)
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

            (_, rc, message) = Pool.Methods.InitCache(
                get_object(pool_object_path), {"devices": [devnodes[2]]}
            )
            self.assertEqual(rc, StratisdErrors.OK, msg=message)

            ((b, _), rc, message) = Manager.Methods.StopPool(
                get_object(TOP_OBJECT),
                {
                    "id": "testpool",
                    "id_type": "name",
                },
            )
            self.assertEqual(b, True)
            self.assertEqual(rc, StratisdErrors.OK, msg=message)

            (_, rc, message) = Manager.Methods.UnsetKey(
                get_object(TOP_OBJECT),
                {
                    "key_desc": "testkey",
                },
            )
            self.assertEqual(rc, StratisdErrors.OK, msg=message)

            (_, rc, _) = Manager.Methods.StartPool(
                get_object(TOP_OBJECT),
                {
                    "id": "testpool",
                    "id_type": "name",
                    "unlock_method": (True, (False, 0)),
                    "key_fd": (False, 0),
                    "remove_cache": False,
                },
            )
            self.assertNotEqual(rc, 0)

            (out_side, in_side) = os.pipe()
            os.write(in_side, b"testkey")
            (_, rc, message) = Manager.Methods.SetKey(
                get_object(TOP_OBJECT),
                {
                    "key_desc": "testkey",
                    "key_fd": out_side,
                },
            )
            self.assertEqual(rc, StratisdErrors.OK, msg=message)

            (_, rc, message) = Manager.Methods.StartPool(
                get_object(TOP_OBJECT),
                {
                    "id": "testpool",
                    "id_type": "name",
                    "unlock_method": (True, (False, 0)),
                    "key_fd": (False, 0),
                    "remove_cache": False,
                },
            )
            self.assertEqual(rc, StratisdErrors.OK, msg=message)


class TestRemoveCache(UdevTest):
    """
    Test creating a pool with a cache and then removing it on start.
    """

    def cache_removal(self, key_spec=None):
        """
        * Create pool
        * Add cache
        * Stop pool
        * Start pool with flag
        * Cache should be removed
        * Readd the cache device that should have been wiped in the last step
        """

        device_tokens = self._lb_mgr.create_devices(3)
        devnodes = self._lb_mgr.device_files(device_tokens)

        with OptionalKeyServiceContextManager(key_spec=key_spec):
            (_, (pool_object_path, _)) = create_pool("testpool", devnodes[:2])
            (_, rc, message) = Pool.Methods.InitCache(
                get_object(pool_object_path), {"devices": [devnodes[2]]}
            )
            self.assertEqual(rc, StratisdErrors.OK, msg=message)

            assert bool(Pool.Properties.HasCache.Get(get_object(pool_object_path)))

            ((b, _), rc, message) = Manager.Methods.StopPool(
                get_object(TOP_OBJECT),
                {
                    "id": "testpool",
                    "id_type": "name",
                },
            )
            self.assertEqual(b, True)
            self.assertEqual(rc, StratisdErrors.OK, msg=message)

            ((b, (pool_object_path, _, _)), rc, message) = Manager.Methods.StartPool(
                get_object(TOP_OBJECT),
                {
                    "id": "testpool",
                    "id_type": "name",
                    "unlock_method": (False, (False, 0)),
                    "key_fd": (False, 0),
                    "remove_cache": True,
                },
            )
            self.assertEqual(rc, StratisdErrors.OK, msg=message)

            assert not bool(Pool.Properties.HasCache.Get(get_object(pool_object_path)))

            (_, rc, message) = Pool.Methods.InitCache(
                get_object(pool_object_path), {"devices": [devnodes[2]]}
            )
            self.assertEqual(rc, StratisdErrors.OK, msg=message)

    def test_unencrypted_cache_removal(self):
        """
        Test removing a cache from an unencrypted pool.
        """
        self.cache_removal()

    def test_encrypted_cache_removal(self):
        """
        Test removing a cache from an encrypted pool.
        """
        self.cache_removal(key_spec=[("testkey", "testkey")])
