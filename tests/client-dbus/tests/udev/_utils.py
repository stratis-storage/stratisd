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
import json
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
from tenacity import (
    RetryError,
    Retrying,
    retry_if_exception_type,
    retry_if_not_result,
    stop_after_attempt,
    stop_after_delay,
    wait_fixed,
)

# isort: LOCAL
from stratisd_client_dbus import (
    Blockdev,
    Manager,
    MOBlockDev,
    MOPool,
    ObjectManager,
    Pool,
    StratisdErrors,
    blockdevs,
    get_object,
    pools,
)
from stratisd_client_dbus._constants import TOP_OBJECT

from ._dm import remove_stratis_setup
from ._loopback import LoopBackDevices

_STRATISD = os.environ["STRATISD"]
STRATISD = _STRATISD
_LEGACY_POOL = os.environ.get("LEGACY_POOL")

CRYPTO_LUKS_FS_TYPE = "crypto_LUKS"
STRATIS_FS_TYPE = "stratis"


def random_string(length):
    """
    Generates a random string with the prefix 'stratis_'
    :param length: Length of random part of string
    :return: String
    """
    return f'stratis_{"".join(random.choice(string.ascii_uppercase) for _ in range(length))}'


# pylint: disable=too-many-statements
def create_pool(
    name, devices, *, key_description=None, clevis_info=None, overprovision=True
):
    """
    Creates a stratis pool.
    :param name:    Name of pool
    :param devices:  Devices to use for pool
    :param key_description: optional key descriptions and token slots
    :type key_description: list of tuples
    :param clevis_info: clevis information, pin and config
    :type clevis_info: list of tuples
    :return: result of pool create if operation succeeds
    :rtype: bool * str * list of str
    :raises RuntimeError: if pool is not created
    """

    def create_legacy_pool():
        newly_created = False

        if len(get_pools(name)) == 0:
            cmdline = [_LEGACY_POOL, name] + devices
            if key_description is not None:
                if len(key_description) > 1:
                    raise RuntimeError(
                        "Can only provide one key description to legacy pools"
                    )
                (kd, _) = key_description[0]
                cmdline.extend(["--key-desc", kd])
            if clevis_info is not None:
                if len(clevis_info) > 1:
                    raise RuntimeError(
                        "Can only provide one Clevis info to legacy pools"
                    )
                (pin, (tang_url, thp), _) = clevis_info[0]
                cmdline.extend(["--clevis", pin])
                if pin == "tang":
                    cmdline.extend(["--tang-url", tang_url])
                    if thp is None:
                        cmdline.append("--trust-url")
                    else:
                        cmdline.extend(["--thumbprint", thp])

            with subprocess.Popen(
                cmdline,
                text=True,
                stdin=subprocess.PIPE,
            ) as process:
                process.stdin.write(  # pyright: ignore [reportOptionalMemberAccess]
                    f"Yes{os.linesep}"
                )
                process.stdin.flush()  # pyright: ignore [reportOptionalMemberAccess]
                process.wait()
                if process.returncode != 0:
                    raise RuntimeError(
                        f"Unable to create a pool {name} with devices {devices}: {process.stderr}"
                    )

            newly_created = True

        i = 0
        while get_pools(name) == [] and i < 5:
            i += 1
            time.sleep(1)
        (pool_object_path, _) = next(iter(get_pools(name)))
        bd_object_paths = [op for op, _ in get_blockdevs(pool_object_path)]

        return (newly_created, (pool_object_path, bd_object_paths))

    def create_v2_pool():
        dbus_key_descriptions = []
        for kd, slot in key_description if key_description is not None else []:
            if slot is None:
                dbus_slot = (False, 0)
            else:
                dbus_slot = (True, slot)

            dbus_key_descriptions.append((dbus_slot, kd))

        dbus_clevis_infos = []
        for pin, (tang_url, thp), slot in (
            clevis_info if clevis_info is not None else []
        ):
            if slot is None:
                dbus_slot = (False, 0)
            else:
                dbus_slot = (True, slot)

            if pin == "tang":
                (pin, config) = (
                    "tang",
                    json.dumps(
                        {"url": tang_url, "stratis:tang:trust_url": True}
                        if thp is None
                        else {"url": tang_url, "thp": thp}
                    ),
                )
            else:
                raise RuntimeError(
                    "Currently only Tang is supported for Clevis in the test infrastructure"
                )

            dbus_clevis_infos.append((dbus_slot, pin, config))

        (result, exit_code, error_str) = Manager.Methods.CreatePool(
            get_object(TOP_OBJECT),
            {
                "name": name,
                "devices": devices,
                "key_desc": dbus_key_descriptions,
                "clevis_info": dbus_clevis_infos,
                "journal_size": (False, 0),
                "tag_spec": (False, ""),
                "allocate_superblock": (False, False),
            },
        )

        if exit_code != StratisdErrors.OK:
            raise RuntimeError(
                f"Unable to create a pool {name} with devices {devices}: {error_str}"
            )

        return result

    (newly_created, (pool_object_path, bd_object_paths)) = (
        create_v2_pool() if _LEGACY_POOL is None else create_legacy_pool()
    )

    if not overprovision:
        Pool.Properties.Overprovisioning.Set(get_object(pool_object_path), False)

    return (newly_created, (pool_object_path, bd_object_paths))


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


def get_blockdevs(pool=None):
    """
    Get the device nodes belonging to the pool indicated by parent.

    :param parent: list of object paths representing blockdevs
    :type blockdev_object_paths: list of str
    :return: list of blockdev information found
    :rtype: list of (str * MOBlockdev)
    """
    managed_objects = ObjectManager.Methods.GetManagedObjects(
        get_object(TOP_OBJECT), {}
    )

    return [
        (op, MOBlockDev(info))
        for op, info in blockdevs(props={} if pool is None else {"Pool": pool}).search(
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
    subprocess.check_call(["/usr/bin/udevadm", "settle"])


def wait_for_udev_count(expected_num):
    """
    Look for devices with ID_FS_TYPE=stratis. Check as many times as can be
    done in 20 seconds or until the number of devices found is equal to the
    number of devices expected. Always get the result of at least 1 enumeration.

    This method should be used only when it is very hard to figure the device
    nodes corresponding to the Stratis block devices.

    :param int expected_num: the number of expected Stratis devices
    :return: None
    :raises RuntimeError: if unexpected number of device nodes is found
    """
    context = pyudev.Context()

    try:
        for attempt in Retrying(
            retry=retry_if_not_result(lambda found_num: found_num == expected_num),
            stop=stop_after_delay(20),
            wait=wait_fixed(1),
        ):
            with attempt:
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
            attempt.retry_state.set_result(found_num)
    except RetryError as err:
        raise RuntimeError(
            "Found unexpected number of devnodes: expected number: "
            f"{expected_num} != found number: {err.last_attempt.result()}"
        ) from err


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
    context = pyudev.Context()

    try:
        for attempt in Retrying(
            retry=retry_if_not_result(
                lambda found_devnodes: found_devnodes == expected_devnodes
            ),
            stop=stop_after_delay(10),
            wait=wait_fixed(1),
        ):
            with attempt:
                found_devnodes = frozenset(
                    [
                        x.device_node
                        for x in context.list_devices(
                            subsystem="block", ID_FS_TYPE=fs_type
                        )
                    ]
                )
            attempt.retry_state.set_result(found_devnodes)

    except RetryError as err:
        raise RuntimeError(
            f'Found unexpected devnodes: expected devnodes: {", ".join(expected_devnodes)} '
            f'!= found_devnodes: {", ".join(err.last_attempt.result())}'
        ) from err


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

    def __init__(self):
        self._service = None

    def start_service(self):
        """
        Starts the stratisd service if it is not already started. Verifies
        that it has not exited at the time the method returns. Verifies that
        the D-Bus service is available.
        """

        settle()

        if next(processes("stratisd"), None) is not None:
            raise RuntimeError("A stratisd process is already running")

        service = subprocess.Popen(  # pylint: disable=consider-using-with
            [x for x in _STRATISD.split(" ") if x != ""],
            text=True,
        )

        try:
            for attempt in Retrying(
                retry=(retry_if_exception_type(dbus.exceptions.DBusException)),
                stop=stop_after_delay(120),
                wait=wait_fixed(0.5),
                reraise=True,
            ):
                if service.poll() is not None:
                    raise RuntimeError(
                        f"Daemon unexpectedly exited with exit code {service.returncode}"
                    )

                with attempt:
                    get_object(TOP_OBJECT)
        except dbus.exceptions.DBusException as err:
            raise RuntimeError(
                "No D-Bus interface for stratisd found although stratisd appears to be running"
            ) from err

        self._service = service
        return self

    def stop_service(self):
        """
        Stops the stratisd daemon previously spawned.
        :return: None
        """
        if self._service is None:
            return

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
        try:
            for attempt in Retrying(
                retry=(
                    retry_if_exception_type(dbus.exceptions.DBusException)
                    | retry_if_not_result(
                        lambda known_pools: len(known_pools) == expected_num
                    )
                ),
                wait=wait_fixed(3),
                stop=stop_after_attempt(expected_num + 1),
                reraise=True,
            ):
                with attempt:
                    known_pools = get_pools(name=name)
                attempt.retry_state.set_result(known_pools)
        except dbus.exceptions.DBusException as err:
            raise RuntimeError(
                "Failed to obtain any information about pools from the D-Bus"
            ) from err
        except RetryError:
            pass

        # At this point, it is not known whether the number of pools is more,
        # fewer, or equal to the number of pools expected. It is only known
        # that the D-Bus is returning a result. If the number of pools is
        # equal to the number of pools expected, that may be due to timing,
        # and be about to change. So, regardless, try to get the pools one
        # more time and verify the correct count.
        time.sleep(3)
        known_pools = get_pools(name=name)
        self.assertEqual(len(known_pools), expected_num)

        return known_pools
