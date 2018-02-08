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
Used to test udev "add" event in stratisd
"""

import unittest

import subprocess
import time
import random
import string
import os

from stratisd_client_dbus import Manager
from stratisd_client_dbus import ObjectManager
from stratisd_client_dbus import get_object
from stratisd_client_dbus import pools
from stratisd_client_dbus import Pool

from stratisd_client_dbus._constants import TOP_OBJECT

from .._loopback import LoopBackDevices
from .._dm import remove_stratis_setup

_STRATISD = os.environ['STRATISD']


def rs(l):
    """
    Generates a random string with the prefix 'stratis_'
    :param l: Length of random part of string
    :return: String
    """
    return 'stratis_{0}'.format(''.join(
        random.choice(string.ascii_uppercase) for _ in range(l)))


class UdevAdd(unittest.TestCase):
    """
    Test udev add event support.
    """

    @staticmethod
    def _create_pool(name, devices):
        """
        Creates a stratis pool
        :param name:    Name of pool
        :param devices:  Devices to use for pool
        :return: Dbus proxy object representing pool.
        """
        ((pool_object_path, _), _, _) = Manager.Methods.CreatePool(
            get_object(TOP_OBJECT),
            {
                'name': name,
                'redundancy': (True, 0),
                'force': False,
                'devices': devices
            }
        )
        return get_object(pool_object_path)

    def _device_files(self, tokens):
        """
        Converts a list of loop back devices to a list of /dev file entries
        :param tokens: Loop back device list
        :return: List of loop back devices
        """
        rc = []
        for t in tokens:
            rc.append(self._lb_mgr.device_file(t))
        return rc

    def setUp(self):
        """
        Common needed things
        """
        self._lb_mgr = LoopBackDevices()
        self.addCleanup(self._clean_up)
        self._service = None

    def _clean_up(self):
        """
        Cleans up the test environment
        :return: None
        """
        self._stop_service_remove_dm_tables()

        # Remove the loop back devices
        if self._lb_mgr:
            self._lb_mgr.destroy_all()
            self._lb_mgr = None

    @staticmethod
    def _get_pools(name=None):
        """
        Returns a list of the pools or a list with 1 element if name is set and
        found, else empty list
        :param name: Optional filter for pool name
        :return:
        """
        managed_objects = ObjectManager.Methods.GetManagedObjects(
            get_object(TOP_OBJECT), {})

        if name:
            return list(pools(managed_objects, {'Name': name}))
        return list(pools(managed_objects, {}))

    def _start_service(self):
        """
        Starts the stratisd service and verifies it's still up and running
        before we return.
        :return: None
        """
        if not self._service:
            self._service = subprocess.Popen([os.path.join(_STRATISD),
                                              '--debug'])

            limit = time.time() + 10.0
            while time.time() <= limit:
                try:
                    get_object(TOP_OBJECT)
                    break
                # pylint: disable=bare-except
                except:
                    time.sleep(0.1)

                    # If service has exited we will bail
                    if self._service.poll() is not None:
                        break

            # see if service process still exists...
            time.sleep(1)
            if self._service.poll() is not None:
                rc = self._service.returncode
                self._service = None
                raise Exception("Daemon unexpectedly exited with %s" % str(rc))

    def _stop_service_remove_dm_tables(self):
        """
        Stops the service and removes any stratis dm table(s)
        :return: None
        """
        if self._service:
            self._service.terminate()
            self._service.wait()
            self._service = None

            remove_stratis_setup()

    @staticmethod
    def _settle():
        """
        Wait until udev add is complete for us.
        :return: None
        """
        # What is the best way to ensure we wait long enough for
        # the event to be done, this seems to work for now.
        # pylint: disable=no-member
        subprocess.run(['udevadm', 'settle'], check=True)
        time.sleep(1)

    # pylint: disable=too-many-locals
    def _test_driver(self, number_of_pools, dev_count_pool,
                     some_existing=False):
        """
        We want to test 1..N number of devices in the following scenarios:

        * Devices with no signatures getting hot-plug
        * 1 or more devices in pool
          - All devices present @ startup
          - 1 or more @ startup, but incomplete number of devices at startup
          - 0 @ startup, systematically adding one @ a time

        :param number_of_pools: Number of pools
        :param dev_count_pool: Number of devices in each pool
        :param some_existing: Hotplug some devices before we start the daemon
        :return: None
        """

        pool_data = {}

        self._start_service()

        # Create the pools
        for p in range(0, number_of_pools):
            device_tokens = []

            for i in range(0, dev_count_pool):
                device_tokens.append(self._lb_mgr.create_device())

            pool_name = rs(5)
            UdevAdd._create_pool(pool_name, self._device_files(device_tokens))
            pool_data[pool_name] = device_tokens

        # Start & Stop the service
        self._stop_service_remove_dm_tables()
        self._start_service()

        # We should have all the devices, so pool should exist after toggle
        self.assertEqual(len(UdevAdd._get_pools()), number_of_pools)

        self._stop_service_remove_dm_tables()

        # Unplug all the devices
        for device_tokens in pool_data.values():
            for d in device_tokens:
                self._lb_mgr.unplug(d)

        self._start_service()

        self.assertEqual(len(UdevAdd._get_pools()), 0)

        # Systematically add a device to each pool, checking that the pool
        # isn't assembled until complete
        activation_sequence = []

        pool_names = pool_data.keys()

        for i in range(0, dev_count_pool):
            for p in pool_names:
                activation_sequence.append(pool_data[p][i])

        for device_token in activation_sequence[:-number_of_pools]:
            self._lb_mgr.hotplug(device_token)

            if some_existing:
                self._stop_service_remove_dm_tables()
                self._start_service()
            else:
                self._settle()
            result = UdevAdd._get_pools()
            self.assertEqual(len(result), 0)

        for device_token in activation_sequence[-number_of_pools:]:
            self._lb_mgr.hotplug(device_token)

        self._settle()
        self.assertEqual(len(UdevAdd._get_pools()), number_of_pools)

        for pn in pool_names:
            self.assertEqual(len(self._get_pools(pn)), 1)

        # After this test we need to clean-up in case we are running again
        # from same test fixture
        self._stop_service_remove_dm_tables()
        self._lb_mgr.destroy_devices()

    def test_combinations(self):
        """
        Test combinations of pools and number of devices in each pool
        :return:
        """
        for pools_num in range(0, 3):
            for device_num in range(1, 4):
                self._test_driver(pools_num, device_num)

    def test_existing(self):
        """
        While we are adding devices back we will stop start the daemon to ensure
        it can start with one or more devices present and complete when the
        other devices come in later.
        :return: None
        """
        self._test_driver(2, 4, True)

    def _single_pool(self, num_devices, num_hotplugs=0):
        """
        Creates a single pool with specified number of devices.
        :param num_devices: Number of devices to use for pool
        :param num_hotplugs: Number of extra udev "add" event per devices
        :return: None
        """
        self._start_service()
        result = UdevAdd._get_pools()
        self.assertEqual(len(result), 0)

        device_tokens = []

        for _ in range(0, num_devices):
            device_tokens.append(self._lb_mgr.create_device())

        self.assertEqual(len(device_tokens), num_devices)

        pool_name = rs(5)
        UdevAdd._create_pool(pool_name, self._device_files(device_tokens))

        self.assertEqual(len(UdevAdd._get_pools()), 1)

        self._stop_service_remove_dm_tables()
        self._start_service()

        # Make sure on a start with all the devices the pool is there!
        self.assertEqual(len(UdevAdd._get_pools()), 1)

        self._stop_service_remove_dm_tables()

        # Remove the devices
        for d in device_tokens:
            self._lb_mgr.unplug(d)

        self._start_service()

        self.assertEqual(len(UdevAdd._get_pools()), 0)

        for d in device_tokens:
            self._lb_mgr.hotplug(d)

        self._settle()

        self.assertEqual(len(UdevAdd._get_pools()), 1)

        # Generate unnecessary hot plug adds
        for _ in range(0, num_hotplugs):
            for d in device_tokens:
                self._lb_mgr.generate_udev_add_event(d)

        self._settle()

        self.assertEqual(len(UdevAdd._get_pools()), 1)

    def test_simultaneous(self):
        """
        Create a single pool with 16 devices and simulate them being hotplug
        at same time
        :return: None
        """
        self._single_pool(16)

    def test_spurious_adds(self):
        """
        Create a single pool with 16 devices and simulate them being hotplug
        at same time and with spurious additional "add" udev events
        :return: None
        """
        self._single_pool(16, 4)

    def test_simple_udev_add(self):
        """
        Create a single pool with 1 device!
        :return: None
        """
        self._single_pool(1, 1)

    def test_duplicate_pool_name(self):
        """
        Create more than one pool with the same name, then dynamically fix it
        :return: None
        """
        pool_name = rs(12)
        pool_tokens = []
        num_pools = 3

        self._start_service()

        # Create some pools with duplicate names
        for i in range(0, num_pools):
            this_pool = []

            for _ in range(0, i + 1):
                this_pool.append(self._lb_mgr.create_device())

            pool_tokens.append(this_pool)
            UdevAdd._create_pool(pool_name, self._device_files(this_pool))

            self._stop_service_remove_dm_tables()
            for d in this_pool:
                self._lb_mgr.unplug(d)

            self._start_service()

        # Hot plug activate each pool in sequence and force a duplicate name
        # error.
        for i in range(0, num_pools):
            for d in pool_tokens[i]:
                self._lb_mgr.hotplug(d)

            self._settle()

            # They all have the same name, so we should only get 1 pool!
            self.assertEqual(len(UdevAdd._get_pools()), 1)

        # Lets dynamically rename the active pools and then hot-plug the other
        # pools so that they all come up.  This simulates what an end user
        # could do to fix this condition until we have CLI support to assist.
        for _ in range(0, num_pools - 1):
            current_pools = UdevAdd._get_pools()

            existing_pool_count = len(current_pools)

            # Change the active pool name to be unique
            for p in current_pools:
                Pool.Methods.SetName(get_object(p[0]), {'name': rs(10)})

            # Generate synthetic add events
            for add_index in range(0, num_pools):
                for d in pool_tokens[add_index]:
                    self._lb_mgr.generate_udev_add_event(d)

            self._settle()
            self.assertEqual(len(UdevAdd._get_pools()), existing_pool_count + 1)

        self.assertEqual(len(UdevAdd._get_pools()), num_pools)
