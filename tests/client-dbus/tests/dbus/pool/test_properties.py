# Copyright 2020 Red Hat, Inc.
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
Test accessing properties of a pool.
"""

# isort: LOCAL
from stratisd_client_dbus import ManagerR1, PoolR1, get_object
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase, device_name_list

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
        proxy = get_object(TOP_OBJECT)
        ((_, (self._pool_object_path, _)), _, _) = ManagerR1.Methods.CreatePool(
            proxy,
            {
                "name": self._POOLNAME,
                "redundancy": (True, 0),
                "devices": _DEVICE_STRATEGY(),
                "keyfile_path": (False, ""),
            },
        )

    def testProps(self):
        """
        Test reading some filesystem properties.
        """
        pool = get_object(self._pool_object_path)
        is_encrypted = PoolR1.Properties.Encrypted.Get(pool)

        self.assertEqual(is_encrypted, False)
