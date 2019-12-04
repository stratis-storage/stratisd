# Copyright 2019 Red Hat, Inc.
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
Test accessing properties of a blockdev.
"""

# isort: STDLIB
from random import choice

# isort: LOCAL
from stratisd_client_dbus import Blockdev, Manager, get_object
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase, device_name_list

_DEVICE_STRATEGY = device_name_list(1)


class SimpleTestCase(SimTestCase):
    """
    Set up a pool with some blockdevs so that their properties can be checked.
    """

    _POOLNAME = "deadpool"

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        super().setUp()
        proxy = get_object(TOP_OBJECT)
        ((_, (_, self._blockdev_object_paths)), _, _) = Manager.Methods.CreatePool(
            proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": _DEVICE_STRATEGY(),
            },
        )

    def testOptionalProps(self):
        """
        Test reading and setting some optional blockdev properties.
        """

        bop = get_object(choice(self._blockdev_object_paths))

        (valid, value) = Blockdev.Properties.HardwareInfo.Get(bop)
        self.assertIsInstance(valid, int)
        self.assertIsInstance(value, str)

        (valid, value) = Blockdev.Properties.UserInfo.Get(bop)
        self.assertIsInstance(valid, int)
        self.assertIsInstance(value, str)

        ((changed, _), rc, _) = Blockdev.Methods.SetUserInfo(
            bop, {"id": (True, "new_id")}
        )

        self.assertEqual(rc, 0)
        self.assertTrue(changed)

        (valid, value) = Blockdev.Properties.UserInfo.Get(bop)
        self.assertEqual(valid, True)
        self.assertEqual(value, "new_id")

        ((changed, _), rc, _) = Blockdev.Methods.SetUserInfo(bop, {"id": (False, "")})

        self.assertEqual(rc, 0)
        self.assertTrue(changed)

        (valid, value) = Blockdev.Properties.UserInfo.Get(bop)
        self.assertEqual(valid, False)
        self.assertIsInstance(value, str)
