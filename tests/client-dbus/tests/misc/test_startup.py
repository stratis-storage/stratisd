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
Test unique stratis instance.
"""


# isort: STDLIB
import os
import subprocess
import unittest

_STRATISD = os.environ["STRATISD"]


class TestUniqueInstance(unittest.TestCase):
    """
    Test that only one instance of stratisd can be running at any given time.
    """

    def setUp(self):
        """
        Start the original stratisd instance. Register a cleanup function to
        terminate it once started.
        """
        self.command_line = [_STRATISD, "--sim", "--debug"]
        self.first_process = subprocess.Popen(
            self.command_line,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            close_fds=True,
            env=os.environ,
        )

        self.addCleanup(self.first_process.terminate)

    def test_unique_instance(self):
        """
        Verify that a second stratisd instance can not be started.
        """
        process = subprocess.Popen(
            self.command_line,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            close_fds=True,
            env=os.environ,
        )
        (_, stderr) = process.communicate()
        self.assertEqual(process.returncode, 1)
        self.assertNotEqual(stderr, "")
