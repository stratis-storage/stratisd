#!/usr/bin/python3
#
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
Common methods for stratis daemon IPC interaction.
"""

import time

from stratisd_client_dbus import Manager
from stratisd_client_dbus import ObjectManager
from stratisd_client_dbus import get_object
from stratisd_client_dbus import pools
from stratisd_client_dbus import Pool

from stratisd_client_dbus._constants import TOP_OBJECT


def pool_create(name, devices):
    """
    Creates a stratis pool
    :param name:    Name of pool
    :param devices:  Devices to use for pool
    :return: Dbus proxy object representing pool.
    """
    # We may be taking too soon to the service and the device(s) may not
    # actually exist, retry on error.
    error_reasons = ""
    for _ in range(3):
        ((pool_object_path, _), exit_code,
         error_str) = Manager.Methods.CreatePool(
             get_object(TOP_OBJECT), {
                 'name': name,
                 'redundancy': (True, 0),
                 'devices': devices
             })
        if int(exit_code) == 0:
            return get_object(pool_object_path)

        error_reasons += "%s " % error_str
        time.sleep(1)

    raise AssertionError("Unable to create a pool %s %s reasons: %s" %
                         (name, str(devices), error_reasons))


def pools_get(name=None):
    """
    Returns a list of the pools or a list with 1 element if name is set and
    found, else empty list
    :param name: Optional filter for pool name
    :return:
    """
    managed_objects = ObjectManager.Methods.GetManagedObjects(
        get_object(TOP_OBJECT), {})

    selector = {} if name is None else {'Name': name}
    return list(pools(props=selector).search(managed_objects))


def pool_name_set(pool, pool_name):
    """
    Sets the name of a pool
    :param pool: Pool abstraction as returned by pools_get
    :param pool_name: New name for pool
    :return: None
    """
    Pool.Methods.SetName(get_object(pool[0]), {'name': pool_name})


def ipc_responding():
    """
    Used to denote if the ipc is available for use, useful when checking
    the daemon for availability.
    :return: True if IPC is responding, else False
    """
    try:
        get_object(TOP_OBJECT)
        return True
    # pylint: disable=bare-except
    except:
        return False
