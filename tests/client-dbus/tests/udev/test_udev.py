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
import unittest

# isort: THIRDPARTY
import psutil

# isort: LOCAL
from stratisd_client_dbus import PoolR1, get_object

from ._loopback import LoopBackDevices
from ._utils import (
    CRYPTO_LUKS_FS_TYPE,
    STRATIS_FS_TYPE,
    KernelKey,
    ServiceContextManager,
    create_pool,
    get_devnodes,
    get_pools,
    processes,
    random_string,
    remove_stratis_dm_devices,
    settle,
    wait_for_udev,
)


class UdevAdd(unittest.TestCase):
    """
    Test udev add event support.
    """

    def setUp(self):
        self._lb_mgr = LoopBackDevices()
        self.addCleanup(self._clean_up)

    def _clean_up(self):
        """
        Cleans up the test environment
        :return: None
        """
        stratisds = list(processes("stratisd"))
        for process in stratisds:
            process.terminate()
        psutil.wait_procs(stratisds)

        remove_stratis_dm_devices()
        self._lb_mgr.destroy_all()

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

                settle()

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
            self.assertEqual(len(get_pools()), number_of_pools)

        remove_stratis_dm_devices()

        self._lb_mgr.unplug(all_tokens)

        wait_for_udev(STRATIS_FS_TYPE, [])

        last_index = dev_count_pool - 1
        with ServiceContextManager():
            self.assertEqual(len(get_pools()), 0)

            # Add all but the last device for each pool
            tokens_to_add = [
                tok
                for device_tokens in pool_data.values()
                for tok in device_tokens[:last_index]
            ]
            self._lb_mgr.hotplug(tokens_to_add)
            wait_for_udev(STRATIS_FS_TYPE, self._lb_mgr.device_files(tokens_to_add))

            self.assertEqual(len(get_pools()), 0)

            # Add the last device that makes each pool complete
            self._lb_mgr.hotplug(
                [device_tokens[last_index] for device_tokens in pool_data.values()]
            )

            wait_for_udev(STRATIS_FS_TYPE, all_devnodes)

            self.assertEqual(len(get_pools()), number_of_pools)

            for name in pool_data:
                self.assertEqual(len(get_pools(name)), 1)

        remove_stratis_dm_devices()

    def test_generic(self):
        """
        See _test_driver for description.
        """
        self._test_driver(2, 4)

    def _single_pool(self, num_devices, *, key_description=None, num_hotplugs=0):
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
        :param key_description: the key description if encrypting the pool
        :type key_description: str or NoneType
        :param int num_hotplugs: Number of synthetic udev "add" event per device
        :return: None
        """
        device_tokens = self._lb_mgr.create_devices(num_devices)
        devnodes = self._lb_mgr.device_files(device_tokens)

        settle()

        with ServiceContextManager():
            self.assertEqual(len(get_pools()), 0)
            (_, (_, device_object_paths)) = create_pool(
                random_string(5), devnodes, key_description=key_description
            )
            self.assertEqual(len(get_pools()), 1)

            self.assertEqual(len(device_object_paths), len(devnodes))
            wait_for_udev(STRATIS_FS_TYPE, get_devnodes(device_object_paths))

        remove_stratis_dm_devices()

        with ServiceContextManager():
            self.assertEqual(len(get_pools()), 1)

        remove_stratis_dm_devices()

        self._lb_mgr.unplug(device_tokens)

        wait_for_udev(STRATIS_FS_TYPE, [])

        with ServiceContextManager():
            self.assertEqual(len(get_pools()), 0)

            self._lb_mgr.hotplug(device_tokens)

            wait_for_udev(
                STRATIS_FS_TYPE if key_description is None else CRYPTO_LUKS_FS_TYPE,
                devnodes,
            )

            self.assertEqual(len(get_pools()), 1)

            for _ in range(num_hotplugs):
                self._lb_mgr.generate_udev_add_events(device_tokens)

            settle()

            self.assertEqual(len(get_pools()), 1)

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

    @unittest.expectedFailure
    def test_encryption_single_pool(self):
        """
        See documentation for _single_pool.
        """
        with KernelKey("test_key") as key_description:
            self._single_pool(1, key_description=key_description)

    def _simple_initial_discovery_test(self, *, key_description=None):
        """
        A simple test of discovery on start up.

        * Create just one pool
        * Stop the daemon
        * Restart the daemon and verify that the pool is found
        """
        num_devices = 3
        device_tokens = self._lb_mgr.create_devices(num_devices)
        devnodes = self._lb_mgr.device_files(device_tokens)

        settle()

        with ServiceContextManager():
            self.assertEqual(len(get_pools()), 0)
            (_, (_, device_object_paths)) = create_pool(
                random_string(5), devnodes, key_description=key_description
            )

            pool_list = get_pools()
            self.assertEqual(len(pool_list), 1)

            _, this_pool = pool_list[0]
            if key_description is None:
                self.assertFalse(this_pool.Encrypted())
            else:
                self.assertTrue(this_pool.Encrypted())

            self.assertEqual(len(device_object_paths), len(devnodes))

            wait_for_udev(STRATIS_FS_TYPE, get_devnodes(device_object_paths))

        with ServiceContextManager():
            self.assertEqual(len(get_pools()), 1)

        remove_stratis_dm_devices()

    def test_encryption_simple_initial_discovery(self):
        """
        See documentation for _simple_initial_discovery_test.
        """
        with KernelKey("test_key") as key_description:
            self._simple_initial_discovery_test(key_description=key_description)

    def test_simple_initial_discovery(self):
        """
        See documentation for _simple_initial_discovery_test.
        """
        self._simple_initial_discovery_test()

    def _simple_event_test(self, *, key_description=None):
        """
        A simple test of event-based discovery.

        * Create just one pool.
        * Stop the daemon.
        * Unplug the devices.
        * Start the daemon.
        * Plug the devices in one by one. The pool should come up when the last
        device is plugged in.
        """
        id_fs_type_param = (
            STRATIS_FS_TYPE if key_description is None else CRYPTO_LUKS_FS_TYPE
        )
        num_devices = 3
        device_tokens = self._lb_mgr.create_devices(num_devices)
        devnodes = self._lb_mgr.device_files(device_tokens)

        settle()

        with ServiceContextManager():
            self.assertEqual(len(get_pools()), 0)
            (_, (_, device_object_paths)) = create_pool(
                random_string(5), devnodes, key_description=key_description
            )
            self.assertEqual(len(get_pools()), 1)
            self.assertEqual(len(device_object_paths), len(devnodes))

        remove_stratis_dm_devices()

        self._lb_mgr.unplug(device_tokens)
        wait_for_udev(id_fs_type_param, [])

        with ServiceContextManager():
            self.assertEqual(len(get_pools()), 0)

            indices = list(range(num_devices))
            random.shuffle(indices)

            tokens_up = []
            for index in indices[:-1]:
                tokens_up.append(device_tokens[index])
                self._lb_mgr.hotplug([tokens_up[-1]])
                wait_for_udev(id_fs_type_param, self._lb_mgr.device_files(tokens_up))
                self.assertEqual(len(get_pools()), 0)

            tokens_up.append(device_tokens[indices[-1]])
            self._lb_mgr.hotplug([tokens_up[-1]])
            wait_for_udev(id_fs_type_param, self._lb_mgr.device_files(tokens_up))
            self.assertEqual(len(get_pools()), 1)

        remove_stratis_dm_devices()

    @unittest.expectedFailure
    def test_encryption_simple_event(self):
        """
        See documentation for _simple_event_test.
        """
        with KernelKey("test_key") as key_description:
            self._simple_event_test(key_description=key_description)

    def test_simple_event(self):
        """
        See documentation for _simple_event_test.
        """
        self._simple_event_test()

    def test_duplicate_pool_name(self):
        """
        Create more than one pool with the same name, then dynamically fix it
        :return: None
        """
        pool_name = random_string(12)
        pool_tokens = []
        num_pools = 3

        # Create some pools with duplicate names
        for i in range(num_pools):
            this_pool = self._lb_mgr.create_devices(i + 1)
            settle()

            pool_tokens.append(this_pool)

            devnodes = self._lb_mgr.device_files(this_pool)
            with ServiceContextManager():
                create_pool(pool_name, devnodes)

            remove_stratis_dm_devices()

            self._lb_mgr.unplug(this_pool)

            wait_for_udev(STRATIS_FS_TYPE, [])

        all_tokens = [dev for sublist in pool_tokens for dev in sublist]

        with ServiceContextManager():
            # Hot plug activate each pool in sequence and force a duplicate name
            # error.
            for i in range(num_pools):
                self._lb_mgr.hotplug(pool_tokens[i])

            wait_for_udev(STRATIS_FS_TYPE, self._lb_mgr.device_files(all_tokens))

            # The number of pools should never exceed one, since all the pools
            # previously formed in the test have the same name.
            self.assertEqual(len(get_pools()), 1)

            # Dynamically rename all active pools to a randomly chosen name,
            # then generate synthetic add events for every loopbacked device.
            # After num_pools - 1 iterations, all pools should have been set up.
            for _ in range(num_pools - 1):
                current_pools = get_pools()

                # Rename all active pools to a randomly selected new name
                for object_path, _ in current_pools:
                    PoolR1.Methods.SetName(
                        get_object(object_path), {"name": random_string(10)}
                    )

                # Generate synthetic add events for every loop backed device
                self._lb_mgr.generate_udev_add_events(
                    [dev for sublist in pool_tokens for dev in sublist]
                )

                settle()

                self.assertEqual(len(get_pools()), len(current_pools) + 1)

            self.assertEqual(len(get_pools()), num_pools)

        remove_stratis_dm_devices()
