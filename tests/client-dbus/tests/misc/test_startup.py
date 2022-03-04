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

# isort: THIRDPARTY
import psutil

_STRATISD = os.environ["STRATISD"]


class TestUniqueInstance(unittest.TestCase):
    """
    Test that only one instance of stratisd can be running at any given time.
    """

    # pylint: disable=consider-using-with
    def setUp(self):
        """
        Start the original stratisd instance. Register a cleanup function to
        terminate it once started.
        """
        process = subprocess.Popen(
            [_STRATISD, "--sim"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            close_fds=True,
            env=os.environ,
        )

        while not psutil.pid_exists(process.pid):
            pass

        def cleanup():
            process.terminate()
            process.wait()

        self.addCleanup(cleanup)

    def test_unique_instance(self):
        """
        Verify that a second stratisd instance can not be started.
        """
        env = dict(os.environ)
        env["RUST_LOG"] = env.get("RUST_LOG", "") + ",nix::fcntl=debug"
        with subprocess.Popen(
            [_STRATISD, "--sim"],
            stderr=subprocess.STDOUT,
            text=True,
            close_fds=True,
            env=env,
        ) as process:
            (_, _) = process.communicate()
            self.assertEqual(process.returncode, 1)
