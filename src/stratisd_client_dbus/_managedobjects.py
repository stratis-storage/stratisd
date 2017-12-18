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
Wrapper for GetManagedObjects() result.
"""

import xml.etree.ElementTree as ET

from dbus_client_gen import managed_object_class
from dbus_client_gen import mo_query_builder

from ._data import SPECS

pools = mo_query_builder(ET.fromstring(SPECS['org.storage.stratis1.pool']))
filesystems = mo_query_builder(ET.fromstring(SPECS['org.storage.stratis1.filesystem']))
blockdevs = mo_query_builder(ET.fromstring(SPECS['org.storage.stratis1.blockdev']))

MOPool = managed_object_class(
   "MOPool",
   ET.fromstring(SPECS['org.storage.stratis1.pool'])
)
MOBlockDev = managed_object_class(
   "MOBlockDev",
   ET.fromstring(SPECS['org.storage.stratis1.blockdev'])
)
