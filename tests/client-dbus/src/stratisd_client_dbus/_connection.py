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
Miscellaneous helpful methods.
"""

# isort: THIRDPARTY
import dbus

from ._constants import SERVICE


class Bus:
    """
    Our bus.
    """

    # pylint: disable=too-few-public-methods

    _BUS = None

    @staticmethod
    def get_bus():
        """
        Get our bus.
        """
        if Bus._BUS is None:
            Bus._BUS = dbus.SystemBus()

        return Bus._BUS


def get_object(object_path):
    """
    Get an object from an object path.

    :param str object_path: an object path with a valid format
    :returns: the proxy object corresponding to the object path
    :rtype: ProxyObject
    """
    return Bus.get_bus().get_object(SERVICE, object_path, introspect=False)
