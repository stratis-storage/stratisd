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

# isort: STDLIB
import os
import random
import string
import subprocess
import time
import unittest

# isort: THIRDPARTY
import psutil

_STRATISD = os.environ["STRATISD"]


def device_name_list(min_devices=0, max_devices=10):
    """
    Return a function that returns a random list of device names based on
    parameters.
    """

    def the_func():
        return [
            "/dev/%s"
            % "".join(
                random.choice(string.ascii_uppercase + string.digits) for _ in range(4)
            )
            for _ in range(random.randrange(min_devices, max_devices + 1))
        ]

    return the_func


class _Service:
    """
    Handle starting and stopping the Rust service.
    """

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._stratisd = subprocess.Popen([_STRATISD, "--sim"])
        time.sleep(1)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._stratisd.terminate()
        self._stratisd.wait()

    def cleanup(self):
        """
        Stop the daemon if it has been started.
        """
        if hasattr(self, "_stratisd"):
            self.tearDown()


class SimTestCase(unittest.TestCase):
    """
    A SimTestCase must always start and stop stratisd (simulator vesion).
    """

    @classmethod
    def setUpClass(cls):
        """
        Assert that there are no other stratisd processes running.
        """
        for pid in psutil.pids():
            try:
                assert psutil.Process(pid).name() != "stratisd", (
                    "Evidently a stratisd process with process id %u is running" % pid
                )
            except psutil.NoSuchProcess:
                pass

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = _Service()
        self.addCleanup(self._service.cleanup)
        self._service.setUp()
