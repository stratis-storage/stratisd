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
Used to test stratisd and devices being removed while in use
"""

import unittest

from ..lib._nbd import flight_check, NbdDevices
from ..lib._stratis import pool_create, pools_get
from ..lib._utils import rs, settle
from ..lib._test_case_base import UdevBase


class UdevRemove(UdevBase):
    """
    Test to ensure stratisd can handle removing a device in use.
    """

    def setUp(self):
        """
        Common needed things
        """
        super(UdevRemove, self).setUp()

        result = flight_check()
        if result:
            return self.skipTest(result)

        self._blk_mgr = NbdDevices()
        return None

    @unittest.expectedFailure
    def test_single_device_in_pool(self):
        """
        Create a pool with a single device and remove the device while in use.
        :return: None
        """

        self._service.start()
        self.assertEqual(len(pools_get()), 0)

        device_token = self._blk_mgr.create_device()

        pool_name = rs(5)
        pool_create(pool_name, [self._blk_mgr.device_file(device_token)])

        self.assertEqual(len(pools_get()), 1)

        self._blk_mgr.unplug(device_token)
        settle()

        self.assertEqual(len(pools_get()), 1)
