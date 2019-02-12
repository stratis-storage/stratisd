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
Misc. utility functions.
"""

import os
import random
import string
import subprocess
import time


def process_exists(name):
    """
    Walk the process table looking for executable 'name', returns pid if one
    found, else return None
    """
    for p in [pid for pid in os.listdir('/proc') if pid.isdigit()]:
        try:
            exe_name = os.readlink(os.path.join("/proc/", p, "exe"))
        except OSError:
            continue
        if exe_name and exe_name.endswith(os.path.join("/", name)):
            return p
    return None


def settle():
    """
    Wait until udev add is complete for us.
    :return: None
    """
    # What is the best way to ensure we wait long enough for
    # the event to be done, this seems to work for now.
    subprocess.check_call(['udevadm', 'settle'])
    time.sleep(2)


def rs(l):
    """
    Generates a random string with the prefix 'stratis_'
    :param l: Length of random part of string
    :return: String
    """
    return 'stratis_{0}'.format(''.join(
        random.choice(string.ascii_uppercase) for _ in range(l)))
