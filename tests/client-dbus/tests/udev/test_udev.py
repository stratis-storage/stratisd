# Copyright 2018 Red Hat, Inc.
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
Used to test behavior of the udev device discovery mechanism.
"""

# isort: STDLIB
import random

# isort: LOCAL
from stratisd_client_dbus import (
    FetchProperties,
    Manager,
    Pool,
    StratisdErrors,
    get_object,
)
from stratisd_client_dbus._constants import TOP_OBJECT
from stratisd_client_dbus._stratisd_constants import EncryptionMethod

from ._loopback import UDEV_ADD_EVENT
from ._utils import (
    CRYPTO_LUKS_FS_TYPE,
    STRATIS_FS_TYPE,
    OptionalKeyServiceContextManager,
    ServiceContextManager,
    UdevTest,
    create_pool,
    get_devnodes,
    random_string,
    remove_stratis_dm_devices,
    settle,
    wait_for_udev,
    wait_for_udev_count,
)

LOCKED_POOL_UUIDS_PROP_NAME = "LockedPoolsWithDevs"


class UdevTest1(UdevTest):
    """
    See description of test in _test_driver method.
    """

    def _test_driver(self, number_of_pools, dev_count_pool):
        """
        Run the following test:

        0. Start stratisd.
        1. Create number_of_pools pools each with dev_count_pool devices.
        2. Stop stratisd and take down all Stratis dm devices.
        3. Verify that the number of devices with Stratis metadata is the
        same as the number of devices used when creating pools.
        4. Start stratisd, verify that it can find the correct number of pools.
        5. Stop stratisd and take down all Stratis dm devices.
        6. Unplug all the loopbacked devices.
        7. Verify that no devices with Stratis metadata can be found.
        8. Start stratisd, verify that no pools are found.
        9. Plug all but the last device for each pool. Verify that stratisd
        reports no pools.
        10. Stop stratisd and restart it. Verify that stratisd reports no pools.
        11. Add the last device for each pool, verify that stratisd detects
        all pools.

        :param int number_of_pools: the number of pools to use in the test
        :param int dev_count_pool: the number of devices per pool
        """

        pool_data = {}
        with ServiceContextManager():
            for _ in range(number_of_pools):
                device_tokens = self._lb_mgr.create_devices(dev_count_pool)

                pool_name = random_string(5)

                create_pool(pool_name, self._lb_mgr.device_files(device_tokens))
                pool_data[pool_name] = device_tokens

        remove_stratis_dm_devices()

        all_tokens = [
            dev for device_tokens in pool_data.values() for dev in device_tokens
        ]
        all_devnodes = self._lb_mgr.device_files(all_tokens)

        wait_for_udev(STRATIS_FS_TYPE, all_devnodes)

        with ServiceContextManager():
            self.wait_for_pools(number_of_pools)

        remove_stratis_dm_devices()

        self._lb_mgr.unplug(all_tokens)

        wait_for_udev(STRATIS_FS_TYPE, [])

        last_index = dev_count_pool - 1
        with ServiceContextManager():
            self.wait_for_pools(0)

            # Add all but the last device for each pool
            tokens_to_add = [
                tok
                for device_tokens in pool_data.values()
                for tok in device_tokens[:last_index]
            ]
            self._lb_mgr.hotplug(tokens_to_add)
            wait_for_udev(STRATIS_FS_TYPE, self._lb_mgr.device_files(tokens_to_add))

            self.wait_for_pools(0)

            # Add the last device that makes each pool complete
            self._lb_mgr.hotplug(
                [device_tokens[last_index] for device_tokens in pool_data.values()]
            )

            wait_for_udev(STRATIS_FS_TYPE, all_devnodes)

            self.wait_for_pools(number_of_pools)

            for name in pool_data:
                self.wait_for_pools(1, name=name)

        remove_stratis_dm_devices()

    def test_generic(self):
        """
        See _test_driver for description.
        """
        self._test_driver(2, 4)


class UdevTest2(UdevTest):
    """
    Exercise a single pool.
    """

    def _single_pool(self, num_devices, *, num_hotplugs=0):
        """
        Creates a single pool with specified number of devices.

        Verifies the following:
        * On service start there are no pools
        * After pool creation there is one pool and all block devices passed
        to the pool creation method have Stratis metadata
        * After the daemon is brought down and restarted it has found a pool
        * After the loop backed devices have been removed no devices with
        Stratis metadata are found and the newly brought up daemon finds 0
        pools.
        * After the devices are re-added, they can all be found with Stratis
        metadata and the daemon has a pool.
        * Causing num_hotplugs synthetic udev events for each device has
        no further effect, i.e., no additional pools suddenly appear.

        :param int num_devices: Number of devices to use for pool
        :param int num_hotplugs: Number of synthetic udev "add" event per device
        :return: None
        """
        device_tokens = self._lb_mgr.create_devices(num_devices)
        devnodes = self._lb_mgr.device_files(device_tokens)

        with ServiceContextManager():
            self.wait_for_pools(0)
            (_, (_, device_object_paths)) = create_pool(random_string(5), devnodes)
            self.wait_for_pools(1)

            self.assertEqual(len(device_object_paths), len(devnodes))
            wait_for_udev(STRATIS_FS_TYPE, get_devnodes(device_object_paths))

        remove_stratis_dm_devices()

        with ServiceContextManager():
            self.wait_for_pools(1)

        remove_stratis_dm_devices()

        self._lb_mgr.unplug(device_tokens)

        wait_for_udev(STRATIS_FS_TYPE, [])

        with ServiceContextManager():
            self.wait_for_pools(0)

            self._lb_mgr.hotplug(device_tokens)

            wait_for_udev(STRATIS_FS_TYPE, devnodes)

            self.wait_for_pools(1)

            for _ in range(num_hotplugs):
                self._lb_mgr.generate_synthetic_udev_events(
                    device_tokens, UDEV_ADD_EVENT
                )

            settle()

            self.wait_for_pools(1)

        remove_stratis_dm_devices()

    def test_simultaneous(self):
        """
        See documentation for _single_pool.
        """
        self._single_pool(8)

    def test_spurious_adds(self):
        """
        See documentation for _single_pool.
        """
        self._single_pool(4, num_hotplugs=4)

    def test_simple_udev_add(self):
        """
        See documentation for _single_pool.
        """
        self._single_pool(1, num_hotplugs=1)


class UdevTest3(UdevTest):
    """
    A very simple test that just creates a pool, and then brings down the
    daemon, brings it up again, and allows it to discover the existing pool.
    """

    def _simple_initial_discovery_test(
        self, *, key_spec=None, take_down_dm=False
    ):  # pylint: disable=too-many-locals
        """
        A simple test of discovery on start up.

        * Create just one pool
        * Stop the daemon
        * Restart the daemon and verify that the pool is found

        :param key_spec: specification for a key to be inserted into the kernel
                         keyring consisting of the key description and key data
        :type key_spec: (str, bytes) or NoneType
        :param bool take_down_dm: if True take down all Stratis devicemapper
        devices once stratisd is shut down
        """
        num_devices = 3
        device_tokens = self._lb_mgr.create_devices(num_devices)
        devnodes = self._lb_mgr.device_files(device_tokens)
        key_spec = None if key_spec is None else [key_spec]

        with OptionalKeyServiceContextManager(key_spec=key_spec) as key_descriptions:
            key_description = None if key_spec is None else key_descriptions[0]

            self.wait_for_pools(0)
            (_, (pool_object_path, device_object_paths)) = create_pool(
                random_string(5), devnodes, key_description=key_description
            )
            pool_uuid = Pool.Properties.Uuid.Get(get_object(pool_object_path))

            self.wait_for_pools(1)

            wait_for_udev(STRATIS_FS_TYPE, get_devnodes(device_object_paths))

        if take_down_dm:
            remove_stratis_dm_devices()

        with OptionalKeyServiceContextManager(key_spec=key_spec):
            ((option, unlock_uuids), exit_code, _) = Manager.Methods.UnlockPool(
                get_object(TOP_OBJECT),
                {
                    "pool_uuid": pool_uuid,
                    "unlock_method": str(EncryptionMethod.KEYRING),
                },
            )
            if key_spec is None:
                self.assertNotEqual(exit_code, StratisdErrors.OK)
                self.assertEqual(option, False)
            else:
                self.assertEqual(exit_code, StratisdErrors.OK)
                self.assertEqual(option, take_down_dm)
                self.assertEqual(len(unlock_uuids), num_devices if take_down_dm else 0)

            wait_for_udev_count(num_devices)

            self.wait_for_pools(1)

        remove_stratis_dm_devices()

    def test_encryption_simple_initial_discovery(self):
        """
        See documentation for _simple_initial_discovery_test.
        """
        self._simple_initial_discovery_test(key_spec=("test_key_desc", "test_key"))

    def test_simple_initial_discovery(self):
        """
        See documentation for _simple_initial_discovery_test.
        """
        self._simple_initial_discovery_test()

    def test_encryption_simple_initial_discovery_with_takedown(self):
        """
        See documentation for _simple_initial_discovery_test.
        """
        self._simple_initial_discovery_test(
            key_spec=("test_key_desc", "test_key"), take_down_dm=True
        )


class UdevTest4(UdevTest):
    """
    A test that verifies successful discovery of devices via udev events.

    A pool is created. Then the daemon is brought down and all Stratis
    devicemapper devices are destroyed and the devices are unplugged.

    Then the daemon is brought back up again. The devices are plugged back
    in, and it is verified that the daemon has recreated the pool.
    """

    def _simple_event_test(self, *, key_spec=None):  # pylint: disable=too-many-locals
        """
        A simple test of event-based discovery.

        * Create just one pool.
        * Stop the daemon.
        * Unplug the devices.
        * Start the daemon.
        * Plug the devices in one by one. The pool should come up when the last
        device is plugged in.

        :param key_spec: specification for a key to be inserted into the kernel
                         keyring consisting of the key description and key data
        :type key_spec: (str, bytes) or NoneType
        """
        num_devices = 3
        udev_wait_type = STRATIS_FS_TYPE if key_spec is None else CRYPTO_LUKS_FS_TYPE
        device_tokens = self._lb_mgr.create_devices(num_devices)
        devnodes = self._lb_mgr.device_files(device_tokens)
        key_spec = None if key_spec is None else [key_spec]

        with OptionalKeyServiceContextManager(key_spec=key_spec) as key_descriptions:
            key_description = None if key_spec is None else key_descriptions[0]

            self.wait_for_pools(0)
            (_, (pool_object_path, _)) = create_pool(
                random_string(5), devnodes, key_description=key_description
            )
            pool_uuid = Pool.Properties.Uuid.Get(get_object(pool_object_path))

            self.wait_for_pools(1)

        remove_stratis_dm_devices()

        self._lb_mgr.unplug(device_tokens)
        wait_for_udev(udev_wait_type, [])

        with OptionalKeyServiceContextManager(key_spec=key_spec):
            self.wait_for_pools(0)

            indices = list(range(num_devices))
            random.shuffle(indices)

            tokens_up = []
            for index in indices[:-1]:
                tokens_up.append(device_tokens[index])
                self._lb_mgr.hotplug([tokens_up[-1]])
                wait_for_udev(udev_wait_type, self._lb_mgr.device_files(tokens_up))
                self.wait_for_pools(0)

            ((option, unlock_uuids), exit_code, _) = Manager.Methods.UnlockPool(
                get_object(TOP_OBJECT),
                {
                    "pool_uuid": pool_uuid,
                    "unlock_method": str(EncryptionMethod.KEYRING),
                },
            )
            if key_spec is None:
                self.assertNotEqual(exit_code, StratisdErrors.OK)
                self.assertEqual(option, False)
            else:
                self.assertEqual(exit_code, StratisdErrors.OK)
                self.assertEqual(option, True)
                self.assertEqual(len(unlock_uuids), num_devices - 1)

            self.wait_for_pools(0)

            tokens_up.append(device_tokens[indices[-1]])
            self._lb_mgr.hotplug([tokens_up[-1]])

            wait_for_udev(udev_wait_type, self._lb_mgr.device_files(tokens_up))

            ((option, unlock_uuids), exit_code, _) = Manager.Methods.UnlockPool(
                get_object(TOP_OBJECT),
                {
                    "pool_uuid": pool_uuid,
                    "unlock_method": str(EncryptionMethod.KEYRING),
                },
            )

            if key_spec is None:
                self.assertNotEqual(exit_code, StratisdErrors.OK)
                self.assertEqual(option, False)
            else:
                self.assertEqual(exit_code, StratisdErrors.OK)
                self.assertEqual(option, True)
                self.assertEqual(len(unlock_uuids), 1)

            wait_for_udev_count(num_devices)

            self.wait_for_pools(1)

        remove_stratis_dm_devices()

    def test_simple_event(self):
        """
        See documentation for _simple_event_test.
        """
        self._simple_event_test()

    def test_encryption_simple_event(self):
        """
        See documentation for _simple_event_test.
        """
        self._simple_event_test(key_spec=("test_key_desc", "test_key"))


class UdevTest5(UdevTest):
    """
    Test correct handling of pools with duplicate pool names.

    This test creates multiple pools with the same name but different UUIDs.
    It is possible to do this by repeatedly bringing up the daemon, creating
    a pool, bringing the daemon down again, and then unplugging the devices
    belonging to that pool. When the daemon comes up again, the previously
    created pool is invisible, and so another of the same name can be created.

    Next, the daemon is brought up and all the previously created devices
    are made visible. Only one pool is set up, the others must all be placed
    in the set of liminal devices, because they represent pools with the same
    name. Then all existing pools are renamed. Then, all devices receive
    synthetic events, which should cause another pool to be discovered, and
    so forth. Eventually, all pools should have been set up.
    """

    def test_duplicate_pool_name(self):  # pylint: disable=too-many-locals
        """
        Create more than one pool with the same name, then dynamically fix it
        :return: None
        """
        pool_name = random_string(12)
        pool_tokens = []
        encrypted_indices = []
        unencrypted_indices = []
        num_pools = 3
        keys = [
            ("key_desc_1", "key_data_1"),
            ("key_desc_2", "key_data_2"),
            ("key_desc_3", "key_data_3"),
        ]

        # Create some pools with duplicate names
        for i in range(num_pools):
            this_pool = self._lb_mgr.create_devices(i + 1)
            devnodes = self._lb_mgr.device_files(this_pool)

            with OptionalKeyServiceContextManager(key_spec=keys) as key_descriptions:
                key_description = (
                    key_descriptions[random.randint(0, len(key_descriptions) - 1)]
                    if random.choice([True, False])
                    else None
                )
                create_pool(pool_name, devnodes, key_description=key_description)
                if key_description is None:
                    unencrypted_indices.append(i)
                else:
                    encrypted_indices.append(i)

            pool_tokens.append(this_pool)

            remove_stratis_dm_devices()

            self._lb_mgr.unplug(this_pool)

            wait_for_udev(STRATIS_FS_TYPE, [])

        all_tokens = [dev for sublist in pool_tokens for dev in sublist]
        random.shuffle(all_tokens)

        with OptionalKeyServiceContextManager(key_spec=keys):
            self._lb_mgr.hotplug(all_tokens)

            (luks_tokens, non_luks_tokens) = (
                [
                    dev
                    for sublist in (pool_tokens[i] for i in encrypted_indices)
                    for dev in sublist
                ],
                [
                    dev
                    for sublist in (pool_tokens[i] for i in unencrypted_indices)
                    for dev in sublist
                ],
            )

            wait_for_udev(CRYPTO_LUKS_FS_TYPE, self._lb_mgr.device_files(luks_tokens))
            wait_for_udev(STRATIS_FS_TYPE, self._lb_mgr.device_files(non_luks_tokens))

            (valid, variant_pool_uuids) = FetchProperties.Methods.GetProperties(
                get_object(TOP_OBJECT), {"properties": [LOCKED_POOL_UUIDS_PROP_NAME]}
            )[LOCKED_POOL_UUIDS_PROP_NAME]

            self.assertTrue(valid)

            for pool_uuid in variant_pool_uuids:
                ((option, _), exit_code, _) = Manager.Methods.UnlockPool(
                    get_object(TOP_OBJECT),
                    {
                        "pool_uuid": pool_uuid,
                        "unlock_method": str(EncryptionMethod.KEYRING),
                    },
                )
                self.assertEqual(exit_code, StratisdErrors.OK)
                self.assertEqual(option, True)

            wait_for_udev_count(len(all_tokens))

            # The number of pools should never exceed one, since all the pools
            # previously formed in the test have the same name.
            self.wait_for_pools(1)

            # Dynamically rename all active pools to a randomly chosen name,
            # then generate synthetic add events for every loopbacked device.
            # After num_pools - 1 iterations, all pools should have been set up.
            for pool_count in range(num_pools - 1):
                current_pools = self.wait_for_pools(pool_count + 1)

                # Rename all active pools to a randomly selected new name
                for object_path, _ in current_pools:
                    Pool.Methods.SetName(
                        get_object(object_path), {"name": random_string(10)}
                    )

                self._lb_mgr.generate_synthetic_udev_events(all_tokens, UDEV_ADD_EVENT)

                settle()

                self.wait_for_pools(len(current_pools) + 1)

            self.wait_for_pools(num_pools)

        remove_stratis_dm_devices()
