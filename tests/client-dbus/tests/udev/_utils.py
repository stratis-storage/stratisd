# Copyright 2020 Red Hat, Inc.
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
Support for testing udev device discovery.
"""

# isort: STDLIB
import logging
import os
import random
import signal
import string
import subprocess
import time
import unittest
from tempfile import NamedTemporaryFile

# isort: THIRDPARTY
import dbus
import psutil
import pyudev

# isort: LOCAL
from stratisd_client_dbus import (
    Blockdev,
    Manager,
    MOPool,
    ObjectManager,
    Pool,
    StratisdErrors,
    get_object,
    pools,
)
from stratisd_client_dbus._constants import TOP_OBJECT

from ._dm import remove_stratis_setup
from ._loopback import LoopBackDevices

_STRATISD = os.environ["STRATISD"]

CRYPTO_LUKS_FS_TYPE = "crypto_LUKS"
STRATIS_FS_TYPE = "stratis"


def random_string(length):
    """
    Generates a random string with the prefix 'stratis_'
    :param length: Length of random part of string
    :return: String
    """
    return f'stratis_{"".join(random.choice(string.ascii_uppercase) for _ in range(length))}'


def create_pool(
    name, devices, *, key_description=None, clevis_info=None, overprovision=True
):
    """
    Creates a stratis pool.
    :param name:    Name of pool
    :param devices:  Devices to use for pool
    :param key_description: optional key description
    :type key_description: str or NoneType
    :param clevis_info: clevis information, pin and config
    :type clevis_info: pair of str * str
    :return: result of the CreatePool D-Bus method call if it succeeds
    :rtype: bool * str * list of str
    :raises RuntimeError: if pool is not created
    """
    (result, exit_code, error_str) = Manager.Methods.CreatePool(
        get_object(TOP_OBJECT),
        {
            "name": name,
            "devices": devices,
            "key_desc": (False, "")
            if key_description is None
            else (True, key_description),
            "clevis_info": (False, ("", ""))
            if clevis_info is None
            else (True, clevis_info),
        },
    )

    if exit_code != StratisdErrors.OK:
        raise RuntimeError(
            f"Unable to create a pool {name} with devices {devices}: {error_str}"
        )

    (_, (pool_object_path, _)) = result

    if not overprovision:
        Pool.Properties.Overprovisioning.Set(get_object(pool_object_path), False)

    return result


def get_pools(name=None):
    """
    Returns a list of all pools found by GetManagedObjects, or a list
    of pools with names matching the specified name, if passed.
    :param name: filter for pool name
    :type name: str or NoneType
    :return: list of pool information found
    :rtype: list of (str * MOPool)
    """
    managed_objects = ObjectManager.Methods.GetManagedObjects(
        get_object(TOP_OBJECT), {}
    )

    return [
        (op, MOPool(info))
        for op, info in pools(props={} if name is None else {"Name": name}).search(
            managed_objects
        )
    ]


def get_devnodes(device_object_paths):
    """
    Get the device nodes belonging to these object paths.

    :param blockdev_object_paths: list of object paths representing blockdevs
    :type blockdev_object_paths: list of str
    :returns: a list of device nodes corresponding to the object paths
    :rtype: list of str
    """
    return [
        Blockdev.Properties.Devnode.Get(get_object(op)) for op in device_object_paths
    ]


def settle():
    """
    Wait some amount and then call udevadm settle.
    :return: None
    """
    time.sleep(2)
    subprocess.check_call(["udevadm", "settle"])


def wait_for_udev_count(expected_num):
    """
    Look for devices with ID_FS_TYPE=stratis. Check as many times as can be
    done in 10 seconds or until the number of devices found is equal to the
    number of devices expected. Always get the result of at least 1 enumeration.

    This method should be used only when it is very hard to figure the device
    nodes corresponding to the Stratis block devices.

    :param int expected_num: the number of expected Stratis devices
    :return: None
    :raises RuntimeError: if unexpected number of device nodes is found
    """
    found_num = None

    context = pyudev.Context()
    end_time = time.time() + 10.0

    while time.time() < end_time and not expected_num == found_num:
        found_num = len(
            frozenset(
                [
                    x.device_node
                    for x in context.list_devices(
                        subsystem="block", ID_FS_TYPE=STRATIS_FS_TYPE
                    )
                ]
            )
        )
        time.sleep(1)

    if expected_num != found_num:
        raise RuntimeError(
            f"Found unexpected number of devnodes: "
            f"expected number: {expected_num} != found number: {found_num}"
        )


def wait_for_udev(fs_type, expected_paths):
    """
    Look for devices with ID_FS_TYPE=fs_type. Check as many times as can be
    done in 10 seconds or until the devices found are equal to the devices
    expected. Always get the result of at least 1 enumeration.
    :param str fs_type: the type to look for ("stratis" or "crypto_LUKS")
    :param expected_paths: devnodes of paths that should belong to Stratis
    :type expected_paths: list of str
    :return: None
    :raises RuntimeError: if unexpected device nodes are found
    """
    expected_devnodes = frozenset((os.path.realpath(x) for x in expected_paths))
    found_devnodes = None

    context = pyudev.Context()
    end_time = time.time() + 10.0

    while time.time() < end_time and not expected_devnodes == found_devnodes:
        found_devnodes = frozenset(
            [
                x.device_node
                for x in context.list_devices(subsystem="block", ID_FS_TYPE=fs_type)
            ]
        )
        time.sleep(1)

    if expected_devnodes != found_devnodes:
        raise RuntimeError(
            f'Found unexpected devnodes: expected devnodes: {", ".join(expected_devnodes)} '
            f'!= found_devnodes: {", ".join(found_devnodes)}'
        )


def processes(name):
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


class _Service:
    """
    Start and stop stratisd.
    """

    # pylint: disable=consider-using-with
    def start_service(self):
        """
        Starts the stratisd service if it is not already started. Verifies
        that it has not exited at the time the method returns. Verifies that
        the D-Bus service is available.
        """

        settle()

        if next(processes("stratisd"), None) is not None:
            raise RuntimeError("A stratisd process is already running")

        service = subprocess.Popen(
            [x for x in _STRATISD.split(" ") if x != ""],
            text=True,
        )

        dbus_interface_present = False
        limit = time.time() + 120.0
        while (
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
            raise RuntimeError(
                f"Daemon unexpectedly exited with exit code {service.returncode}"
            )

        if not dbus_interface_present:
            raise RuntimeError("No D-Bus interface for stratisd found")

        self._service = service  # pylint: disable=attribute-defined-outside-init
        return self

    def stop_service(self):
        """
        Stops the stratisd daemon previously spawned.
        :return: None
        """
        self._service.send_signal(signal.SIGINT)
        self._service.wait(timeout=30)
        if next(processes("stratisd"), None) is not None:
            raise RuntimeError("Failed to stop stratisd service")


class KernelKey:
    """
    A handle for operating on keys in the kernel keyring. The specified keys
    will be available for the lifetime of the test when used with the Python
    with keyword and will be cleaned up at the end of the scope of the with
    block.
    """

    def __init__(self, key_descs):
        """
        Initialize a key with the provided key description and key data (passphrase).
        :param key_descs: list of key descriptions, may be empty
        :type key_descs: list of (str * bytes)
        """
        self._key_descs = key_descs

    def __enter__(self):
        """
        This method allows KernelKey to be used with the "with" keyword.
        :return: The key descriptions that can be used to access the
                 provided key data in __init__.
        :raises RuntimeError: if setting a key in the keyring through stratisd
                              fails
        """
        for key_desc, key_data in self._key_descs:
            with NamedTemporaryFile(mode="w") as temp_file:
                temp_file.write(key_data)
                temp_file.flush()

                with open(temp_file.name, "r", encoding="utf-8") as fd_for_dbus:
                    (_, return_code, message) = Manager.Methods.SetKey(
                        get_object(TOP_OBJECT),
                        {
                            "key_desc": key_desc,
                            "key_fd": fd_for_dbus.fileno(),
                        },
                    )

                if return_code != StratisdErrors.OK:
                    raise RuntimeError(
                        f"Setting a key using stratisd failed with an error: {message}"
                    )

        return [desc for (desc, _) in self._key_descs]

    def __exit__(self, exception_type, exception_value, traceback):
        try:
            for key_desc, _ in reversed(self._key_descs):
                (_, return_code, message) = Manager.Methods.UnsetKey(
                    get_object(TOP_OBJECT), {"key_desc": key_desc}
                )

                if return_code != StratisdErrors.OK:
                    raise RuntimeError(
                        f"Unsetting the key using stratisd failed with an error: {message}"
                    )

        except RuntimeError as rexc:
            if exception_value is None:
                raise rexc
            raise rexc from exception_value


class ServiceContextManager:
    """
    A context manager for starting and stopping the daemon.
    """

    def __init__(self):
        self._service = _Service()

    def __enter__(self):
        self._service.start_service()

    def __exit__(self, exc_type, exc_val, exc_tb):
        self._service.stop_service()

        return False


class OptionalKeyServiceContextManager:
    """
    A service context manager that accepts an optional key
    """

    def __init__(self, *, key_spec=None):
        """
        Initialize a context manager with an optional list of keys
        :param key_spec: Key description and data for kernel keys to be added
        :type key_spec: list of (str, str) or NoneType
        """
        self._ctxt_manager = ServiceContextManager()
        self._keys = KernelKey([]) if key_spec is None else KernelKey(key_spec)

    def __enter__(self):
        """
        Chain ServiceContextManager and KernelKey __enter__ methods
        :return: list of key descriptions
        :rtype: list of str
        """
        self._ctxt_manager.__enter__()
        return self._keys.__enter__()

    def __exit__(self, exc_type, exc_val, exc_tb):
        self._keys.__exit__(exc_type, exc_val, exc_tb)
        self._ctxt_manager.__exit__(exc_type, exc_val, exc_tb)


class UdevTest(unittest.TestCase):
    """
    Do some setup and teardown of loopbacked devices and what not.
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
            logging.warning("stratisd process %s still running, terminating", process)
            process.terminate()
        (_, alive) = psutil.wait_procs(stratisds, timeout=10)
        for process in alive:
            logging.warning(
                "stratisd process %s did not respond to terminate signal, killing",
                process,
            )
            process.kill()

        remove_stratis_setup()
        self._lb_mgr.destroy_all()

    def wait_for_pools(self, expected_num, *, name=None):
        """
        Returns a list of all pools found by GetManagedObjects, or a list
        of pools with names matching the specified name, if passed.
        Tries multiple times to get the list of pools via GetManagedObjects
        call. Catches the D-Bus error, just in case there are strange behaviors
        on the test machine, but re-raises the exception if still failing after
        tries. Uses a count instead of a timer, because the D-Bus call
        has a built in timer of its own. Still sleeps after each try in case
        the reason the expected number does not match the real number is that
        the pools have not been brought up yet.
        :param int expected_num: the number of pools expected
        :param name: filter for pool name
        :type name: str or NoneType
        :return: list of pool information found
        :rtype: list of (str * MOPool)
        """
        (count, limit, dbus_err, found_num, known_pools, start_time) = (
            0,
            expected_num + 1,
            None,
            None,
            None,
            time.time(),
        )
        while count < limit and not expected_num == found_num:
            try:
                known_pools = get_pools(name=name)
            except dbus.exceptions.DBusException as err:
                dbus_err = err

            if known_pools is not None:
                found_num = len(known_pools)

            time.sleep(3)
            count += 1

        if found_num is None and dbus_err is not None:
            raise RuntimeError(
                f"After {time.time() - start_time:.2f} seconds, the only "
                "response is a D-Bus exception"
            ) from dbus_err

        self.assertEqual(found_num, expected_num)

        return known_pools
