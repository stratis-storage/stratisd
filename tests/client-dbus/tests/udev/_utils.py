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
import os
import random
import signal
import string
import subprocess
import sys
import time
from tempfile import NamedTemporaryFile

# isort: THIRDPARTY
import dbus
import psutil
import pyudev

# isort: LOCAL
from stratisd_client_dbus import (
    Blockdev,
    ManagerR1,
    MOPoolR1,
    ObjectManager,
    StratisdErrors,
    get_object,
    pools,
)
from stratisd_client_dbus._constants import TOP_OBJECT

from ._dm import _get_stratis_devices, remove_stratis_setup

_STRATISD = os.environ["STRATISD"]

CRYPTO_LUKS_FS_TYPE = "crypto_LUKS"
STRATIS_FS_TYPE = "stratis"


def random_string(length):
    """
    Generates a random string with the prefix 'stratis_'
    :param length: Length of random part of string
    :return: String
    """
    return "stratis_{0}".format(
        "".join(random.choice(string.ascii_uppercase) for _ in range(length))
    )


def create_pool(name, devices, *, key_description=None):
    """
    Creates a stratis pool.
    :param name:    Name of pool
    :param devices:  Devices to use for pool
    :param key_description: optional key description
    :type key_description: str or NoneType
    :return: result of the CreatePool D-Bus method call if it succeeds
    :rtype: bool * str * list of str
    :raises RuntimeError: if pool is not created
    """
    (result, exit_code, error_str) = ManagerR1.Methods.CreatePool(
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
        return result

    raise RuntimeError(
        "Unable to create a pool %s with devices %s: %s" % (name, devices, error_str)
    )


def get_pools(name=None):
    """
    Returns a list of all pools found by GetManagedObjects, or a list
    of pools with names matching the specified name, if passed.
    :param name: filter for pool name
    :type name: str or NoneType
    :return: list of pool information found
    :rtype: list of (str * MOPoolR1)
    """
    managed_objects = ObjectManager.Methods.GetManagedObjects(
        get_object(TOP_OBJECT), {}
    )

    return [
        (op, MOPoolR1(info))
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
            "Found unexpected number of devnodes: expected number: %s != found number: %s"
            % (expected_num, found_num)
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
            "Found unexpected devnodes: expected devnodes: %s != found_devnodes: %s"
            % (", ".join(expected_devnodes), ", ".join(found_devnodes))
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


def remove_stratis_dm_devices():
    """
    Remove Stratis device mapper devices, fail with a runtime error if
    some have been missed.
    :raises RuntimeError: if some devices are remaining
    """
    remove_stratis_setup()
    if _get_stratis_devices() != []:
        raise RuntimeError("Some devices were not removed")


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

        settle()

        if list(processes("stratisd")) != []:
            raise RuntimeError("A stratisd process is already running")

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
            raise RuntimeError(
                "Daemon unexpectedly exited with %s, stdout: %s, stderr: %s"
                % (
                    service.returncode,
                    service.stdout.readlines(),
                    service.stderr.readlines(),
                )
            )

        if not dbus_interface_present:
            raise RuntimeError("No D-Bus interface for stratisd found")

        self._service = service  # pylint: disable=attribute-defined-outside-init
        return self

    def stop_service(self):
        """
        Stops the stratisd daemon previously spawned.
        :return: a tuple of stdout and stderr
        """
        self._service.send_signal(signal.SIGINT)
        output = self._service.communicate()
        if list(processes("stratisd")) != []:
            raise RuntimeError("Failed to stop stratisd service")

        return output


class KernelKey:
    """
    A handle for operating on keys in the kernel keyring. The specified key will
    be available for the lifetime of the test when used with the Python with
    keyword and will be cleaned up at the end of the scope of the with block.
    """

    def __init__(self, key_desc, key_data):
        """
        Initialize a key with the provided key description and key data (passphrase).
        :param str key_desc: The desired key description
        :param bytes key_data: The desired key contents
        """
        self._key_desc = key_desc
        self._key_data = key_data

    def __enter__(self):
        """
        This method allows KernelKey to be used with the "with" keyword.
        :return: The key description that can be used to access the
                 provided key data in __init__.
        :raises RuntimeError: if setting the key in the keyring through stratisd
                              fails
        """
        with NamedTemporaryFile(mode="w") as temp_file:
            temp_file.write(self._key_data)
            temp_file.flush()

            with open(temp_file.name, "r") as fd_for_dbus:
                (_, return_code, message) = ManagerR1.Methods.SetKey(
                    get_object(TOP_OBJECT),
                    {
                        "key_desc": self._key_desc,
                        "key_fd": fd_for_dbus.fileno(),
                        "interactive": False,
                    },
                )

        if return_code != StratisdErrors.OK:
            raise RuntimeError(
                "Setting the key using stratisd failed with an error: %s" % message
            )

        return self._key_desc

    def __exit__(self, exception_type, exception_value, traceback):
        message = None
        try:
            (_, return_code, message) = ManagerR1.Methods.UnsetKey(
                get_object(TOP_OBJECT), {"key_desc": self._key_desc}
            )

            if return_code != StratisdErrors.OK:
                raise RuntimeError(
                    "Unsetting the key using stratisd failed with an error: %s"
                    % message
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
        (_, stderrdata) = self._service.stop_service()

        print("", file=sys.stdout, flush=True)
        print(
            "Log output from this invocation of stratisd:", file=sys.stdout, flush=True
        )
        print(stderrdata, file=sys.stdout, flush=True)

        return False


class OptionalKeyServiceContextManager:
    """
    A service context manager that accepts an optional key
    """

    def __init__(self, *, key_spec=None):
        """
        Initialize a context manager with an optional key.
        :param key_spec: Key description and key data for the kernel key to be added
        :type key_spec: (str, bytes) or NoneType
        """
        self._ctxt_manager = ServiceContextManager()
        self._key = None if key_spec is None else KernelKey(key_spec[0], key_spec[1])

    def __enter__(self):
        """
        Chain ServiceContextManager and KernelKey __enter__ methods
        :return: key description or None if no key data was provided
        :rtype: str or NoneType
        """
        self._ctxt_manager.__enter__()
        return None if self._key is None else self._key.__enter__()

    def __exit__(self, exc_type, exc_val, exc_tb):
        if self._key is not None:
            self._key.__exit__(exc_type, exc_val, exc_tb)
        self._ctxt_manager.__exit__(exc_type, exc_val, exc_tb)
