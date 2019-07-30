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

from stratisd_client_dbus import Filesystem
from stratisd_client_dbus import Manager
from stratisd_client_dbus import Pool
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase
from .._misc import device_name_list

_DEVICE_STRATEGY = device_name_list()


class SetNameTestCase(SimTestCase):
    """
    Set up a pool with a name and one filesystem.
    """

    _POOLNAME = "deadpool"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        self._fs_name = "fs"
        self._proxy = get_object(TOP_OBJECT)
        ((_, (self._pool_object_path, _)), _, _) = Manager.Methods.CreatePool(
            self._proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": _DEVICE_STRATEGY(),
            },
        )
        self._pool_object = get_object(self._pool_object_path)
        ((_, created), _, _) = Pool.Methods.CreateFilesystems(
            self._pool_object, {"specs": [self._fs_name]}
        )
        self._filesystem_object_path = created[0][0]
        Manager.Methods.ConfigureSimulator(self._proxy, {"denominator": 8})

    def testProps(self):
        """
        Test reading some filesystem properties.
        """
        filesystem = get_object(self._filesystem_object_path)
        name = Filesystem.Properties.Name.Get(filesystem)

        self.assertEqual(self._fs_name, name)

        uuid = Filesystem.Properties.Uuid.Get(filesystem)

        # must be a 32 character string
        self.assertEqual(32, len(uuid))

        created = Filesystem.Properties.Created.Get(filesystem)

        # Should be a UTC rfc3339 string, which should end in Z
        self.assertTrue(created.endswith("Z"))
        # I think this is also always true
        self.assertEqual(len(created), 20)

        used = Filesystem.Properties.Used.Get(filesystem)

        self.assertEqual(used, "12345678")

        devnode = Filesystem.Properties.Devnode.Get(filesystem)

        self.assertEqual(devnode, "/stratis/deadpool/fs")
