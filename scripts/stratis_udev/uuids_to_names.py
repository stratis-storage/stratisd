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
Get pool and filesystem names from UUIDs
"""
import dbus
import re
import sys
import time

_STRATIS_BUS_NAME = "org.storage.stratis2"
_STRATIS_MANAGER_OBJECT = "/org/storage/stratis2"
_STRATIS_POOL_IFACE = "org.storage.stratis2.pool.r1"
_STRATIS_FS_IFACE = "org.storage.stratis2.filesystem"
try:
    _DBUS = dbus.SystemBus()
except:
    print("Failed to connect to system D-Bus")
    sys.exit(1)

def _udev_name_to_uuids(name):
    match_result = re.match("stratis-1-([0-9a-f]{32})-thin-fs-([0-9a-f]{32})", name)
    match_groups = match_result.groups()
    return (match_groups[0], match_groups[1])

def _get_managed_objects():
    top_object = _DBUS.get_object(
        _STRATIS_BUS_NAME,
        _STRATIS_MANAGER_OBJECT,
    )
    interface = dbus.Interface(top_object, "org.freedesktop.DBus.ObjectManager")
    return interface.GetManagedObjects()

def _pool_uuid_to_stratis_name(managed_objects, pool_uuid):
    for obj in managed_objects.values():
        if _STRATIS_POOL_IFACE in obj and obj[_STRATIS_POOL_IFACE]["Uuid"] == pool_uuid:
            return obj[_STRATIS_POOL_IFACE]["Name"]

def _fs_uuid_to_stratis_name(managed_objects, fs_uuid):
    for obj in managed_objects.values():
        if _STRATIS_FS_IFACE in obj and obj[_STRATIS_FS_IFACE]["Uuid"] == fs_uuid:
            return obj[_STRATIS_FS_IFACE]["Name"]

def main():
    if len(sys.argv) != 2:
        print("Thinly provisioned filesystem devicemapper name required")
        sys.exit(1)

    thin_filesystem_name = sys.argv[1]

    (pool_uuid, fs_uuid) = _udev_name_to_uuids(thin_filesystem_name)
    managed_objects = _get_managed_objects()
    pool_name = _pool_uuid_to_stratis_name(managed_objects, pool_uuid)
    fs_name = _fs_uuid_to_stratis_name(managed_objects, fs_uuid)
    output = "%s %s" % (pool_name, fs_name)
    print(output)

if __name__ == "__main__":
    main()
