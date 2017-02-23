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

from ._implementation import ObjectManager

_SERVICE_NAME = "org.storage.stratis1"
_POOL_INTERFACE_NAME = "%s.%s" % (_SERVICE_NAME, "pool")
_FILESYSTEM_INTERFACE_NAME = "%s.%s" % (_SERVICE_NAME, "filesystem")

_POOL_INTERFACE_PROPS = frozenset(("Name", "Uuid"))

class ManagedObjects(object):
    """
    Wraps the dict returned by GetManagedObjects() method with some
    methods.
    """
    # pylint: disable=too-few-public-methods


    def __init__(self, objects): # pragma: no cover
        """
        Initializer.

        :param dict objects: the GetManagedObjects result.
        """
        self._objects = objects

    def pools(self, spec=None): # pragma: no cover
        """
        Get the subset of data corresponding to pools and matching spec.

        :param spec: a specification of properties to restrict values returned
        :type spec: dict of str * object
        :returns: a list of pairs of object path/dict for pools only
        :rtype: list of tuple of ObjectPath * dict

        A match requires a conjunction of all specified properties.
        An empty spec results in all pool objects being returned.
        """
        spec = dict() if spec is None else spec
        interface_name = _POOL_INTERFACE_NAME
        return (
           (op, data) for (op, data) in self._objects.items() \
               if interface_name in data.keys() and \
               all(data[interface_name][key] == value \
                   for (key, value) in spec.items())
        )

    def filesystems(self): # pragma: no cover
        """
        Get the subset of data corresponding to filesystems.

        :returns: a list of dictionaries for pools
        :rtype: list of tuple of ObjectPath * dict
        """
        interface_name = _FILESYSTEM_INTERFACE_NAME
        return (
           (x, y) for (x, y) in self._objects.items() \
               if interface_name in y.keys()
        )


def get_managed_objects(proxy): # pragma: no cover
    """
    Convenience function for managed objects.
    :param proxy: proxy for the manager object
    :returns: a constructed ManagedObjects object
    :rtype: ManagedObjects
    """
    return ManagedObjects(ObjectManager.GetManagedObjects(proxy))
