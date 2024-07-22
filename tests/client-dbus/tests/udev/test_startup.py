# Copyright 2021 Red Hat, Inc.
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
Test starting additional stratisd processes.
"""

# isort: STDLIB
import os
import subprocess
import time

from ._utils import STRATISD, ServiceContextManager, UdevTest


class TestStarting(UdevTest):
    """
    Test starting stratisd when an instance of stratisd is already running.
    """

    def test_unique_instance(self):
        """
        Verify that a second stratisd instance can not be started.
        """
        with ServiceContextManager():
            stratisd_lock_file = "/run/stratisd.pid"

            for _ in range(5):
                if os.path.exists(stratisd_lock_file):
                    break
                time.sleep(1)
            else:
                raise RuntimeError(
                    f"Lock file {stratisd_lock_file} does not seem to exist."
                )

            env = dict(os.environ)
            env["RUST_LOG"] = env.get("RUST_LOG", "") + ",nix::fcntl=debug"
            with subprocess.Popen(
                [STRATISD],
                stderr=subprocess.STDOUT,
                text=True,
                close_fds=True,
                env=env,
            ) as process:
                (_, _) = process.communicate()
                self.assertEqual(process.returncode, 1)
