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
import os
import random
import string
import subprocess
import time
import unittest

# isort: THIRDPARTY
import dbus
import psutil
import pyudev

# isort: LOCAL
from stratisd_client_dbus import (
    Manager,
    ObjectManager,
    Pool,
    StratisdErrors,
    get_object,
    pools,
)
from stratisd_client_dbus._constants import TOP_OBJECT

from ._dm import _get_stratis_devices, remove_stratis_setup
from ._loopback import LoopBackDevices
from ._stratis_id import dump_stratis_signature_area, stratis_signature

_STRATISD = os.environ["STRATISD"]


def rs(l):
    """
    Generates a random string with the prefix 'stratis_'
    :param l: Length of random part of string
    :return: String
    """
    return "stratis_{0}".format(
        "".join(random.choice(string.ascii_uppercase) for _ in range(l))
    )


class UdevAdd(unittest.TestCase):
    """
    Test udev add event support.
    """

    @staticmethod
    def _create_pool(name, devices):
        """
        Creates a stratis pool. Tries three times before giving up.
        Raises an assertion error if it does not succeed after three tries.
        :param name:    Name of pool
        :param devices:  Devices to use for pool
        :return: Dbus proxy object representing pool.
        """
        error_reasons = []
        for _ in range(3):
            (
                (_, (pool_object_path, _)),
                exit_code,
                error_str,
            ) = Manager.Methods.CreatePool(
                get_object(TOP_OBJECT),
                {"name": name, "redundancy": (True, 0), "devices": devices},
            )
            if exit_code == StratisdErrors.OK:
                return get_object(pool_object_path)

            error_reasons.append(error_str)
            time.sleep(1)

        raise AssertionError(
            "Unable to create a pool %s %s reasons: %s" % (name, devices, error_reasons)
        )

    def _device_files(self, tokens):
        """
        Converts a list of loop back devices to a list of /dev file entries
        :param tokens: list of UUIDs
        :type tokens: list of uuid.UUID
        :return: List of device nodes corresponding to tokens, may include None
        :rtype: list of (str or NoneType)
        """
        return [self._lb_mgr.device_file(t) for t in tokens]

    def setUp(self):
        self._lb_mgr = LoopBackDevices()
        self.addCleanup(self._clean_up)

    def _clean_up(self):
        """
        Cleans up the test environment
        :return: None
        """
        self._stop_service_remove_dm_tables()
        self._lb_mgr.destroy_all()

    @staticmethod
    def _get_pools(name=None):
        """
        Returns a list of the pools or a list with 1 element if name is set and
        found, else empty list
        :param name: Optional filter for pool name
        :return:
        """
        managed_objects = ObjectManager.Methods.GetManagedObjects(
            get_object(TOP_OBJECT), {}
        )

        selector = {} if name is None else {"Name": name}
        return list(pools(props=selector).search(managed_objects))

    def _start_service(self):
        """
        Starts the stratisd service if it is not already started. Verifies
        that it has not exited at the time the method returns. Verifies that
        the D-Bus service is available.
        :return: None
        """

        if getattr(self, "_service", None) is None:
            self._settle()

            assert UdevAdd._process_exists("stratisd") is None
            assert _get_stratis_devices() == []

            self._service = subprocess.Popen(  # pylint: disable=attribute-defined-outside-init
                [_STRATISD, "--debug"]
            )

            dbus_interface_present = False
            limit = time.time() + 120.0
            while (  # pylint: disable=bad-continuation
                time.time() <= limit
                and not dbus_interface_present
                and self._service.poll() is None
            ):
                try:
                    get_object(TOP_OBJECT)
                    dbus_interface_present = True
                except dbus.exceptions.DBusException:
                    time.sleep(0.5)

            time.sleep(1)
            if self._service.poll() is not None:
                rc = self._service.returncode
                self._service = None  # pylint: disable=attribute-defined-outside-init
                raise Exception("Daemon unexpectedly exited with %s" % rc)

            assert dbus_interface_present, "No D-Bus interface for stratisd found"

    def _stop_service_remove_dm_tables(self):
        """
        Stops the service and removes any stratis dm table(s)
        :return: None
        """
        if getattr(self, "_service", None) is not None:
            self._service.terminate()
            self._service.wait()
            self._service = None  # pylint: disable=attribute-defined-outside-init

            assert UdevAdd._process_exists("stratisd") is None

            remove_stratis_setup()
            assert _get_stratis_devices() == []

    @staticmethod
    def _settle():
        """
        Wait some amount and then call udevadm settle.
        :return: None
        """
        time.sleep(2)
        subprocess.check_call(["udevadm", "settle"])

    @staticmethod
    def dump_state(context, expected_paths):
        """
        Dump everything we can when we are missing stratis devices!
        :param context:  udev context
        :param expected_paths: list of devices which we know should have
               signatures
        :return: None
        """
        print("We expect Stratis signatures on %d device(s)" % len(expected_paths))
        for d in expected_paths:
            signature = stratis_signature(d)
            print("%s shows signature check of %s" % (d, signature))

            if signature is None:
                # We are really expecting this device to have the signature
                # lets dump the signature area of the disk
                dump_stratis_signature_area(d)

        print("Udev db dump of all block devices")
        for d in context.list_devices(subsystem="block"):
            for k, v in d.items():
                print("%s:%s" % (k, str(v)))
            print("")

    @staticmethod
    def _expected_stratis_block_devices(num_expected, expected_paths):
        """
        Look for Stratis devices. Check as many times as can be done in
        10 seconds or until the number of devices found equals the number
        of devices expected. Always get the result of at least 1 Stratis
        enumeration.
        :param int num_expected: number of expected devnodes
        :param expected_paths: devnodes of paths that should belong to Stratis
        :type expected_paths: list of str
        :return: None (May assert)
        """

        assert num_expected == len(expected_paths)

        found = None
        context = pyudev.Context()
        end_time = time.time() + 10.0

        while time.time() < end_time and found != num_expected:
            found = sum(
                1 for _ in context.list_devices(subsystem="block", ID_FS_TYPE="stratis")
            )
            time.sleep(1)

        if found != num_expected:
            UdevAdd.dump_state(context, expected_paths)

        assert found == num_expected

    @staticmethod
    def _process_exists(name):
        """
        Look through processes, using their pids, to find one matching 'name'.
        Return None if no such process found, else return the pid.
        :param name: name of process to check
        :type name: str
        :return: pid or None
        :rtype: int or NoneType
        """
        for proc in psutil.process_iter(["name"]):
            try:
                if proc.name() == name:
                    return proc.pid
            except psutil.NoSuchProcess:
                pass
        return None

    def _test_driver(  # pylint: disable=bad-continuation
        self, number_of_pools, dev_count_pool, some_existing=False
    ):  # pylint: disable=too-many-locals
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

        expected_stratis_devices = []

        # Create the pools
        for _ in range(number_of_pools):
            device_tokens = [
                self._lb_mgr.create_device() for _ in range(dev_count_pool)
            ]
            self._settle()

            pool_name = rs(5)
            devnodes = self._device_files(device_tokens)

            UdevAdd._create_pool(pool_name, devnodes)
            pool_data[pool_name] = device_tokens
            expected_stratis_devices.extend(devnodes)

        # Start & Stop the service
        self._stop_service_remove_dm_tables()

        UdevAdd._expected_stratis_block_devices(
            dev_count_pool * number_of_pools, expected_stratis_devices
        )

        self._start_service()

        # We should have all the devices, so pool should exist after toggle
        self.assertEqual(len(UdevAdd._get_pools()), number_of_pools)

        self._stop_service_remove_dm_tables()

        # Unplug all the devices
        for device_tokens in pool_data.values():
            for d in device_tokens:
                self._lb_mgr.unplug(d)

        UdevAdd._expected_stratis_block_devices(0, [])

        self._start_service()

        self.assertEqual(len(UdevAdd._get_pools()), 0)

        # Systematically add a device to each pool, checking that the pool
        # isn't assembled until complete
        pool_names = pool_data.keys()

        activation_sequence = [
            pool_data[p][i] for i in range(dev_count_pool) for p in pool_names
        ]

        # Add all but the last device for each pool
        running_count = 0
        running_devices = []

        for device_token in activation_sequence[:-number_of_pools]:
            self._lb_mgr.hotplug(device_token)
            running_count += 1
            running_devices.extend(self._device_files([device_token]))

            UdevAdd._expected_stratis_block_devices(running_count, running_devices)

            if some_existing:
                self._stop_service_remove_dm_tables()
                self._start_service()
            else:
                self._settle()
            self.assertEqual(len(UdevAdd._get_pools()), 0)

        # Add the last device that makes each pool complete
        for device_token in activation_sequence[-number_of_pools:]:
            self._lb_mgr.hotplug(device_token)

        UdevAdd._expected_stratis_block_devices(
            number_of_pools * dev_count_pool, expected_stratis_devices
        )

        self._settle()
        self.assertEqual(len(UdevAdd._get_pools()), number_of_pools)

        for pn in pool_names:
            self.assertEqual(len(self._get_pools(pn)), 1)

        # After this test we need to clean-up in case we are running again
        # from same test fixture
        self._stop_service_remove_dm_tables()
        self._lb_mgr.destroy_devices()
        UdevAdd._expected_stratis_block_devices(0, [])

    def test_combinations(self):
        """
        Test combinations of pools and number of devices in each pool
        :return:
        """
        for pools_num in range(3):
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
        self._start_service()
        self.assertEqual(len(UdevAdd._get_pools()), 0)

        device_tokens = [self._lb_mgr.create_device() for _ in range(num_devices)]
        devnodes = self._device_files(device_tokens)

        self._settle()

        pool_name = rs(5)
        UdevAdd._create_pool(pool_name, devnodes)
        self.assertEqual(len(UdevAdd._get_pools()), 1)

        self._stop_service_remove_dm_tables()

        UdevAdd._expected_stratis_block_devices(num_devices, devnodes)

        self._start_service()

        self.assertEqual(len(UdevAdd._get_pools()), 1)

        self._stop_service_remove_dm_tables()

        for d in device_tokens:
            self._lb_mgr.unplug(d)
        UdevAdd._expected_stratis_block_devices(0, [])

        self._start_service()

        self.assertEqual(len(UdevAdd._get_pools()), 0)

        for d in device_tokens:
            self._lb_mgr.hotplug(d)

        self._settle()
        UdevAdd._expected_stratis_block_devices(num_devices, devnodes)

        self.assertEqual(len(UdevAdd._get_pools()), 1)

        for _ in range(num_hotplugs):
            for d in device_tokens:
                self._lb_mgr.generate_udev_add_event(d)

        self._settle()
        UdevAdd._expected_stratis_block_devices(num_devices, devnodes)

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

        # Create some pools with duplicate names
        for i in range(num_pools):
            self._start_service()

            this_pool = [self._lb_mgr.create_device() for _ in range(i + 1)]

            self._settle()

            pool_tokens.append(this_pool)
            devices = self._device_files(this_pool)
            UdevAdd._create_pool(pool_name, devices)

            self._stop_service_remove_dm_tables()

            UdevAdd._expected_stratis_block_devices(len(this_pool), devices)

            for d in this_pool:
                self._lb_mgr.unplug(d)

            UdevAdd._expected_stratis_block_devices(0, [])

        self._start_service()

        # Hot plug activate each pool in sequence and force a duplicate name
        # error.
        plugged = 0
        devices_plugged = []
        for i in range(num_pools):
            for d in pool_tokens[i]:
                self._lb_mgr.hotplug(d)
                plugged += 1
                devices_plugged.extend(self._device_files([d]))

            self._settle()
            UdevAdd._expected_stratis_block_devices(plugged, devices_plugged)

            # The number of pools should never exceed one, since all the pools
            # previously formed in the test have the same name.
            self.assertEqual(len(UdevAdd._get_pools()), 1)

        # Dynamically rename all active pools to a randomly chosen name,
        # then generate synthetic add events for every loopbacked device.
        # After num_pools - 1 iterations, all pools should have been set up.
        for _ in range(num_pools - 1):
            current_pools = UdevAdd._get_pools()

            # Rename all active pools to a randomly selected new name
            for object_path, _ in current_pools:
                Pool.Methods.SetName(get_object(object_path), {"name": rs(10)})

            # Generate synthetic add events for every loop backed device
            for d in (d for sublist in pool_tokens for d in sublist):
                self._lb_mgr.generate_udev_add_event(d)

            self._settle()
            UdevAdd._expected_stratis_block_devices(plugged, devices_plugged)
            self.assertEqual(len(UdevAdd._get_pools()), len(current_pools) + 1)

        self.assertEqual(len(UdevAdd._get_pools()), num_pools)
