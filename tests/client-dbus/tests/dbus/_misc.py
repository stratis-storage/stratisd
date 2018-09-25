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
from pathlib import Path
import string
import subprocess
import time

from hypothesis import strategies

_STRATISD = os.environ['STRATISD']


def _device_list(minimum):
    """
    Get a device generating strategy.

    :param int minimum: the minimum number of devices, must be at least 0
    """
    return strategies.lists(
        strategies.text(alphabet=string.ascii_letters + "/", min_size=1),
        min_size=minimum)


def stratisd_bin(given_binary):
    """
    See if we can locate binary in alternative location, we are
    assuming that it's in either a debug or release sub dir.  This prevents
    us from needing to change the build env. on CI servers.
    :param given_binary:
    :return:
    """
    if Path(given_binary).exists():
        return given_binary

    if "debug/stratisd" in given_binary:
        alternative = given_binary.replace("debug/stratisd",
                                           "release/stratisd", 1)
    else:
        alternative = given_binary.replace("release/stratisd",
                                           "debug/stratisd", 1)

    return alternative


class Service():
    """
    Handle starting and stopping the Rust service.
    """

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        executable = stratisd_bin(_STRATISD)

        self._stratisd = subprocess.Popen([os.path.join(executable), '--sim'])

        time.sleep(1)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        # pylint: disable=no-member
        self._stratisd.terminate()
        self._stratisd.wait()
