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
Test 'stratisd'.
"""

import time
import unittest

from stratisd_client_dbus import Manager
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import Service


class StratisTestCase(unittest.TestCase):
    """
    Test meta information about stratisd.
    """

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testStratisVersion(self):
        """
        Getting version should just succeed.
        """
        result = Manager.getProperty(get_object(TOP_OBJECT), "Version")
        self.assertIsInstance(result, str)

    @unittest.expectedFailure
    def testStratisLogLevel(self):
        """
        Getting log level should just succeed.
        """
        result = Manager.getProperty(get_object(TOP_OBJECT), "LogLevel")
        self.assertIsInstance(result, int)

    def testStratisLogLevel1(self):
        """
        Getting log level property does get a value.
        """
        result = Manager.getProperty(get_object(TOP_OBJECT), "LogLevel")
        self.assertIsNotNone(result)
