# Copyright 2016 Red Hat, Inc.
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
Classes to implement dbus interface.
"""

# isort: STDLIB
import xml.etree.ElementTree as ET

# isort: FIRSTPARTY
from dbus_client_gen import managed_object_class, mo_query_builder
from dbus_python_client_gen import make_class

from ._data import SPECS

_POOL_SPEC = ET.fromstring(SPECS["org.storage.stratis2.pool.r3"])
_FILESYSTEM_SPEC = ET.fromstring(SPECS["org.storage.stratis2.filesystem"])
_BLOCKDEV_SPEC = ET.fromstring(SPECS["org.storage.stratis2.blockdev.r2"])

pools = mo_query_builder(_POOL_SPEC)
filesystems = mo_query_builder(_FILESYSTEM_SPEC)
blockdevs = mo_query_builder(_BLOCKDEV_SPEC)

MOPool = managed_object_class("MOPool", _POOL_SPEC)
MOBlockDev = managed_object_class("MOBlockDev", _BLOCKDEV_SPEC)

TIME_OUT = 120  # In seconds

ObjectManager = make_class(
    "ObjectManager",
    ET.fromstring(SPECS["org.freedesktop.DBus.ObjectManager"]),
    TIME_OUT,
)
Report = make_class(
    "Report", ET.fromstring(SPECS["org.storage.stratis2.Report.r1"]), TIME_OUT
)
Manager = make_class(
    "Manager", ET.fromstring(SPECS["org.storage.stratis2.Manager.r3"]), TIME_OUT
)
FetchProperties = make_class(
    "FetchProperties",
    ET.fromstring(SPECS["org.storage.stratis2.FetchProperties.r3"]),
    TIME_OUT,
)
Filesystem = make_class("Filesystem", _FILESYSTEM_SPEC, TIME_OUT)
Pool = make_class("Pool", _POOL_SPEC, TIME_OUT)
Blockdev = make_class("Blockdev", _BLOCKDEV_SPEC, TIME_OUT)
