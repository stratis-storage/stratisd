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
Test accessing properties of a filesystem.
"""

# isort: LOCAL
from stratisd_client_dbus import Filesystem, Manager, Pool, get_object
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase, device_name_list

_DEVICE_STRATEGY = device_name_list()


class SetNameTestCase(SimTestCase):
    """
    Set up a pool with a name and one filesystem.
    """

    _POOLNAME = "deadpool"
    _FSNAME = "fs"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        proxy = get_object(TOP_OBJECT)
        ((_, (pool_object_path, _)), _, _) = Manager.Methods.CreatePool(
            proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": _DEVICE_STRATEGY(),
            },
        )
        pool_object = get_object(pool_object_path)
        ((_, created), _, _) = Pool.Methods.CreateFilesystems(
            pool_object, {"specs": [self._FSNAME]}
        )
        self._filesystem_object_path = created[0][0]
        Manager.Methods.ConfigureSimulator(proxy, {"denominator": 8})

    def testProps(self):
        """
        Test reading some filesystem properties.
        """
        filesystem = get_object(self._filesystem_object_path)
        name = Filesystem.Properties.Name.Get(filesystem)

        self.assertEqual(self._FSNAME, name)

        uuid = Filesystem.Properties.Uuid.Get(filesystem)

        # must be a 32 character string
        self.assertEqual(32, len(uuid))

        created = Filesystem.Properties.Created.Get(filesystem)

        # Should be a UTC rfc3339 string, which should end in Z
        self.assertTrue(created.endswith("Z"))
        # I think this is also always true
        self.assertEqual(len(created), 20)

        devnode = Filesystem.Properties.Devnode.Get(filesystem)

        self.assertEqual(devnode, "/stratis/deadpool/fs")
