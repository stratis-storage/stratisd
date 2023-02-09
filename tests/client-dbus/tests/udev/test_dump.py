# Copyright 2022 Red Hat, Inc.
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
Test that stratis-dumpmetadata can do its job.
"""

# isort: STDLIB
import os
import subprocess

from ._utils import ServiceContextManager, UdevTest, create_pool, random_string

_STRATIS_DUMPMETADATA = os.environ["STRATIS_DUMPMETADATA"]


def _call_stratis_dumpmetadata(dev, *, print_bytes=False):
    """
    Call stratis-dumpmetadata and return exit code.

    :param str dev: path to Stratis device
    :param bool print_bytes: if true, print bytes also
    """
    with subprocess.Popen(
        [_STRATIS_DUMPMETADATA, f"{dev}"] + (["--print-bytes"] if print_bytes else []),
        stdout=subprocess.PIPE,
    ) as command:
        _, errs = command.communicate()
        exit_code = command.returncode
        if exit_code != 0:
            raise RuntimeError(
                f"Invocation of {_STRATIS_DUMPMETADATA} returned an error: "
                f"{command.returncode}, {errs}"
            )
        return exit_code


class TestDumpMetadata(UdevTest):
    """
    Test that dumping of metadata does occur.
    """

    def test_call(self):
        """
        Verify that stratis-dumpmetadata can run on a device.
        """
        device_tokens = self._lb_mgr.create_devices(4)
        devnodes = self._lb_mgr.device_files(device_tokens)

        with ServiceContextManager():
            pool_name = random_string(5)
            create_pool(pool_name, devnodes)
            self.wait_for_pools(1)
            self.assertEqual(_call_stratis_dumpmetadata(devnodes[0]), 0)

    def test_printbytes_call(self):
        """
        Verify that stratis-dumpmetadata can run on a device with print-bytes
        option set.
        """
        device_tokens = self._lb_mgr.create_devices(4)
        devnodes = self._lb_mgr.device_files(device_tokens)

        with ServiceContextManager():
            pool_name = random_string(5)
            create_pool(pool_name, devnodes)
            self.wait_for_pools(1)
            self.assertEqual(
                _call_stratis_dumpmetadata(devnodes[0], print_bytes=True), 0
            )
