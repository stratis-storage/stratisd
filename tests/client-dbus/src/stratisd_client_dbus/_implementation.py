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

import xml.etree.ElementTree as ET

from dbus_python_client_gen import make_class

from ._data import SPECS

TIME_OUT = 120  # In seconds

ObjectManager = make_class("ObjectManager",
                           ET.fromstring(
                               SPECS['org.freedesktop.DBus.ObjectManager']),
                           TIME_OUT)
Manager = make_class("Manager",
                     ET.fromstring(SPECS['org.storage.stratis1.Manager']),
                     TIME_OUT)
Filesystem = make_class("Filesystem",
                        ET.fromstring(
                            SPECS['org.storage.stratis1.filesystem']),
                        TIME_OUT)
Pool = make_class("Pool", ET.fromstring(SPECS['org.storage.stratis1.pool']),
                  TIME_OUT)
