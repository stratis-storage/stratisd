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
Miscellaneous methods to support testing.
"""

import os
import random
import string
import subprocess
import time
import unittest

_STRATISD = os.environ['STRATISD']


def device_name_list(min_devices=0, max_devices=10):
    """
    Return a function that returns a random list of device names based on
    parameters.
    """

    def the_func():
        return [
            "/dev/%s" % ''.join(
                random.choice(string.ascii_uppercase + string.digits)
                for _ in range(4))
            for _ in range(random.randrange(min_devices, max_devices + 1))
        ]

    return the_func


class _Service():
    """
    Handle starting and stopping the Rust service.
    """

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._stratisd = subprocess.Popen([_STRATISD, '--sim'])
        time.sleep(1)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        # pylint: disable=no-member
        self._stratisd.terminate()
        self._stratisd.wait()


class SimTestCase(unittest.TestCase):
    """
    A SimTestCase must always start and stop stratisd (simulator vesion).
    """

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = _Service()
        self._service.setUp()

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()
