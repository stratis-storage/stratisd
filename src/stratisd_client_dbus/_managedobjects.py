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

class GMOPool(object):
    """
    The D-Bus pool.
    """

    def __init__(self, table): #pragma: no cover
        """
        Initializes the pool with a table.
        """
        self._table = table

    def name(self): #pragma: no cover
        """
        Get the pool name from the table.
        """
        return self._table[_POOL_INTERFACE_NAME]['Name']

    def uuid(self): #pragma: no cover
        """
        Get the pool name from the table.
        """
        return self._table[_POOL_INTERFACE_NAME]['Uuid']

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

    def pools(self): # pragma: no cover
        """
        Get the subset of data corresponding to pools

        :returns: a list of pairs of object path/dict for pools only
        :rtype: list of tuple of ObjectPath * dict
        """
        interface_name = _POOL_INTERFACE_NAME
        return (
           (x, y) for (x, y) in self._objects.items() \
               if interface_name in y.keys()
        )

    def get_pool_by_name(self, name): # pragma: no cover
        """
        Get a single pool for the given name.

        :param str name: the name of the pool
        :returns: a pool object path or None if no pool
        :rtype: str or NoneType
        """
        interface_name = _POOL_INTERFACE_NAME
        pools = (obj_path for (obj_path, data) in self.pools() \
           if data[interface_name]['Name'] == name)
        return next(pools, None)

    def get_pool_by_uuid(self, name): # pragma: no cover
        """
        Get a single pool for the given uuid.

        :param str uuid: the name of the pool
        :returns: a pool object path or None if no pool
        :rtype: str or NoneType
        """
        interface_name = _POOL_INTERFACE_NAME
        pools = (obj_path for (obj_path, data) in self.pools() \
           if data[interface_name]['Uuid'] == name)
        return next(pools, None)

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
