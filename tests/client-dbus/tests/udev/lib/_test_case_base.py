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
Base class for udev tests to reduce code duplication
"""

import unittest

from ..lib._daemon import Daemon
from ..lib._stratis import ipc_responding
from ..lib._udev import StratisBlockDevices


class UdevBase(unittest.TestCase):
    """
    Common unit test code for udev testing
    """

    def setUp(self):
        """
        Common needed things
        """
        self._blk_mgr = None
        self._udev_base_cleanup = True
        self.addCleanup(self._clean_up)
        self._service = Daemon(ipc_responding)
        self._stratis_block_devices = StratisBlockDevices()

    def tearDown(self):
        # We made it here so we are going to do the teardown
        try:
            self._clean_up()
        finally:
            self._udev_base_cleanup = False

    def _clean_up(self):
        """
        Cleans up the test environment
        :return: None
        """
        if self._udev_base_cleanup:
            stop_clean = None

            try:
                self._service.stop_remove_dm_tables()
            # pylint: disable=broad-except
            except Exception as e:
                stop_clean = e

            # Remove the loop back devices
            if self._blk_mgr:
                self._blk_mgr.destroy_all()
                self._blk_mgr = None

            if stop_clean:
                raise stop_clean
