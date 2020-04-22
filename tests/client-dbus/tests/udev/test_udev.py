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
import base64
import os
import random
import shutil
import signal
import string
import subprocess
import sys
import time
import unittest

# isort: THIRDPARTY
import dbus
import psutil
import pyudev

# isort: LOCAL
from stratisd_client_dbus import (
    ManagerR1,
    ObjectManager,
    PoolR1,
    StratisdErrors,
    get_object,
    pools,
)
from stratisd_client_dbus._constants import TOP_OBJECT

from ._dm import _get_stratis_devices, remove_stratis_setup
from ._loopback import LoopBackDevices

_STRATISD = os.environ["STRATISD"]


def random_string(length):
    """
    Generates a random string with the prefix 'stratis_'
    :param length: Length of random part of string
    :return: String
    """
    return "stratis_{0}".format(
        "".join(random.choice(string.ascii_uppercase) for _ in range(length))
    )


def _create_pool(name, devices, *, key_description=None):
    """
    Creates a stratis pool. Tries three times before giving up.
    Raises an assertion error if it does not succeed after three tries.
    :param name:    Name of pool
    :param devices:  Devices to use for pool
    :param key_description: optional key description
    :type key_description: str or NoneType
    :return: Dbus proxy object representing pool.
    """
    error_reasons = []
    for _ in range(3):
        (
            (_, (pool_object_path, _)),
            exit_code,
            error_str,
        ) = ManagerR1.Methods.CreatePool(
            get_object(TOP_OBJECT),
            {
                "name": name,
                "redundancy": (True, 0),
                "devices": devices,
                # pylint: disable=bad-continuation
                "key_desc": (False, "")
                if key_description is None
                else (True, key_description),
            },
        )
        if exit_code == StratisdErrors.OK:
            return get_object(pool_object_path)

        error_reasons.append(error_str)
        time.sleep(1)

    raise AssertionError(
        "Unable to create a pool %s %s reasons: %s" % (name, devices, error_reasons)
    )


def _get_pools(name=None):
    """
    Returns a list of all pools found by GetManagedObjects, or a list
    of pools with names matching the specified name, if passed.
    :param name: filter for pool name
    :type name: str or NoneType
    :return: list of pool information found
    :rtype: list of (str * dict)
    """
    managed_objects = ObjectManager.Methods.GetManagedObjects(
        get_object(TOP_OBJECT), {}
    )

    return list(
        pools(props={} if name is None else {"Name": name}).search(managed_objects)
    )


def _settle():
    """
    Wait some amount and then call udevadm settle.
    :return: None
    """
    time.sleep(2)
    subprocess.check_call(["udevadm", "settle"])


def _wait_for_udev(expected_paths):
    """
    Look for Stratis devices. Check as many times as can be done in
    10 seconds or until the devices found are equal to the devices
    expected. Always get the result of at least 1 Stratis enumeration.
    :param expected_paths: devnodes of paths that should belong to Stratis
    :type expected_paths: list of str
    :return: None (May assert)
    """

    expected_devnodes = frozenset(expected_paths)
    found_devnodes = None

    context = pyudev.Context()
    end_time = time.time() + 10.0

    while time.time() < end_time and not expected_devnodes == found_devnodes:
        found_devnodes = frozenset(
            [
                x.device_node
                for x in context.list_devices(subsystem="block", ID_FS_TYPE="stratis")
            ]
        )
        time.sleep(1)

    assert expected_devnodes == found_devnodes


def _processes(name):
    """
    Find all process matching the given name.
    :param str name: name of process to check
    :return: sequence of psutil.Process
    """
    for proc in psutil.process_iter(["name"]):
        try:
            if proc.name() == name:
                yield proc
        except psutil.NoSuchProcess:
            pass


def _remove_stratis_dm_devices():
    """
    Remove Stratis device mapper devices, fail with an assertion error if
    some have been missed.
    """
    remove_stratis_setup()
    assert _get_stratis_devices() == []


class _Service:
    """
    Start and stop stratisd.
    """

    def start_service(self):
        """
        Starts the stratisd service if it is not already started. Verifies
        that it has not exited at the time the method returns. Verifies that
        the D-Bus service is available.
        """

        _settle()

        assert list(_processes("stratisd")) == []
        assert _get_stratis_devices() == []

        service = subprocess.Popen(
            [_STRATISD],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            universal_newlines=True,
        )

        dbus_interface_present = False
        limit = time.time() + 120.0
        while (  # pylint: disable=bad-continuation
            time.time() <= limit
            and not dbus_interface_present
            and service.poll() is None
        ):
            try:
                get_object(TOP_OBJECT)
                dbus_interface_present = True
            except dbus.exceptions.DBusException:
                time.sleep(0.5)

        time.sleep(1)
        if service.poll() is not None:
            raise Exception("Daemon unexpectedly exited with %s" % service.returncode)

        assert dbus_interface_present, "No D-Bus interface for stratisd found"
        self._service = service  # pylint: disable=attribute-defined-outside-init
        return self

    def stop_service(self):
        """
        Stops the stratisd daemon previously spawned.
        :return: a tuple of stdout and stderr
        """
        self._service.send_signal(signal.SIGINT)
        output = self._service.communicate()
        assert list(_processes("stratisd")) == []
        return output


class _KernelKey:  # pylint: disable=attribute-defined-outside-init
    """
    A handle for operating on keys in the kernel keyring. The specified key will
    be available for the lifetime of the test when used with the Python with
    keyword and will be cleaned up at the end of the scope of the with block.
    """

    def __init__(self, key_data):
        """
        Initialize a key with the provided key data (passphrase).
        :param bytes key_data: The desired key contents
        :raises RuntimeError: if the keyctl command is not found in $PATH
                              or a keyctl command returns a non-zero exit code
        """
        if shutil.which("keyctl") is None:
            raise RuntimeError("Executable keyctl was not found in $PATH")

        self.key_data = key_data

    @staticmethod
    def _raise_keyctl_error(return_code, args):
        """
        Raise an error if keyctl failed to complete an operation
        successfully.
        :param int return_code: Return code of the keyctl command
        :param args: The command line that caused the command to fail
        :type args: list of str
        :raises RuntimeError
        """
        if return_code != 0:
            raise RuntimeError(
                "Command '%s' failed with exit code %s" % (" ".join(args), return_code)
            )

    def __enter__(self):
        """
        This method allows _KernelKey to be used with the "with" keyword.
        :return: The key description that can be used to access the
                 provided key data in __init__.
        """
        with open("/dev/urandom", "rb") as urandom_f:
            key_desc = base64.b64encode(urandom_f.read(16)).decode("utf-8")

        args = ["keyctl", "get_persistent", "@s", "0"]
        exit_values = subprocess.run(args, capture_output=True, text=True)
        _KernelKey._raise_keyctl_error(exit_values.returncode, args)

        self.persistent_id = exit_values.stdout.strip()

        args = ["keyctl", "add", "user", key_desc, self.key_data, self.persistent_id]
        exit_values = subprocess.run(args, capture_output=True)
        _KernelKey._raise_keyctl_error(exit_values.returncode, args)

        return key_desc

    def __exit__(self, exception_type, exception_value, traceback):
        try:
            args = ["keyctl", "clear", self.persistent_id]
            exit_values = subprocess.run(args)
            _KernelKey._raise_keyctl_error(exit_values.returncode, args)

            args = ["keyctl", "clear", "@s"]
            exit_values = subprocess.run(args)
            _KernelKey._raise_keyctl_error(exit_values.returncode, args)
        except RuntimeError as rexc:
            if exception_value is None:
                raise rexc
            raise rexc from exception_value


class _ServiceContextManager:  # pylint: disable=too-few-public-methods
    """
    A context manager for starting and stopping the daemon and cleaning up
    the devicemapper devices it has created.
    """

    def __init__(self):
        self._service = _Service()

    def __enter__(self):
        self._service.start_service()

    def __exit__(self, exc_type, exc_val, exc_tb):
        (_, stderrdata) = self._service.stop_service()

        print("", file=sys.stdout, flush=True)
        print(
            "Log output from this invocation of stratisd:", file=sys.stdout, flush=True
        )
        print(stderrdata, file=sys.stdout, flush=True)

        _remove_stratis_dm_devices()
        return False


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
        stratisds = list(_processes("stratisd"))
        for process in stratisds:
            process.terminate()
        psutil.wait_procs(stratisds)

        _remove_stratis_dm_devices()
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
        with _ServiceContextManager():
            for _ in range(number_of_pools):
                device_tokens = self._lb_mgr.create_devices(dev_count_pool)

                _settle()

                pool_name = random_string(5)

                _create_pool(pool_name, self._lb_mgr.device_files(device_tokens))
                pool_data[pool_name] = device_tokens

        all_tokens = [
            dev for device_tokens in pool_data.values() for dev in device_tokens
        ]
        all_devnodes = self._lb_mgr.device_files(all_tokens)

        _wait_for_udev(all_devnodes)

        with _ServiceContextManager():
            self.assertEqual(len(_get_pools()), number_of_pools)

        self._lb_mgr.unplug(all_tokens)

        _wait_for_udev([])

        last_index = dev_count_pool - 1
        with _ServiceContextManager():
            self.assertEqual(len(_get_pools()), 0)

            # Add all but the last device for each pool
            tokens_to_add = [
                tok
                for device_tokens in pool_data.values()
                for tok in device_tokens[:last_index]
            ]
            self._lb_mgr.hotplug(tokens_to_add)
            _wait_for_udev(self._lb_mgr.device_files(tokens_to_add))

            self.assertEqual(len(_get_pools()), 0)

            # Add the last device that makes each pool complete
            self._lb_mgr.hotplug(
                [device_tokens[last_index] for device_tokens in pool_data.values()]
            )

            _wait_for_udev(all_devnodes)

            self.assertEqual(len(_get_pools()), number_of_pools)

            for name in pool_data:
                self.assertEqual(len(_get_pools(name)), 1)

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

        _settle()

        with _ServiceContextManager():
            self.assertEqual(len(_get_pools()), 0)
            pool_name = random_string(5)
            _create_pool(pool_name, devnodes, key_description=key_description)
            self.assertEqual(len(_get_pools()), 1)

        _wait_for_udev(devnodes)

        with _ServiceContextManager():
            self.assertEqual(len(_get_pools()), 1)

        self._lb_mgr.unplug(device_tokens)

        _wait_for_udev([])

        with _ServiceContextManager():
            self.assertEqual(len(_get_pools()), 0)

            self._lb_mgr.hotplug(device_tokens)

            _wait_for_udev(devnodes)

            self.assertEqual(len(_get_pools()), 1)

            for _ in range(num_hotplugs):
                self._lb_mgr.generate_udev_add_events(device_tokens)

            _settle()

            self.assertEqual(len(_get_pools()), 1)

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
            _settle()

            pool_tokens.append(this_pool)

            devnodes = self._lb_mgr.device_files(this_pool)
            with _ServiceContextManager():
                _create_pool(pool_name, devnodes)

            _wait_for_udev(devnodes)

            self._lb_mgr.unplug(this_pool)

            _wait_for_udev([])

        all_tokens = [dev for sublist in pool_tokens for dev in sublist]

        with _ServiceContextManager():
            # Hot plug activate each pool in sequence and force a duplicate name
            # error.
            for i in range(num_pools):
                self._lb_mgr.hotplug(pool_tokens[i])

            _wait_for_udev(self._lb_mgr.device_files(all_tokens))

            # The number of pools should never exceed one, since all the pools
            # previously formed in the test have the same name.
            self.assertEqual(len(_get_pools()), 1)

            # Dynamically rename all active pools to a randomly chosen name,
            # then generate synthetic add events for every loopbacked device.
            # After num_pools - 1 iterations, all pools should have been set up.
            for _ in range(num_pools - 1):
                current_pools = _get_pools()

                # Rename all active pools to a randomly selected new name
                for object_path, _ in current_pools:
                    PoolR1.Methods.SetName(
                        get_object(object_path), {"name": random_string(10)}
                    )

                # Generate synthetic add events for every loop backed device
                self._lb_mgr.generate_udev_add_events(
                    [dev for sublist in pool_tokens for dev in sublist]
                )

                _settle()

                self.assertEqual(len(_get_pools()), len(current_pools) + 1)

            self.assertEqual(len(_get_pools()), num_pools)
