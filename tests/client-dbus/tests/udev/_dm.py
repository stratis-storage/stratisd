# Copyright 2018 Red Hat, Inc.
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
Functionality pertaining to device mapper
"""

# isort: STDLIB
import os
import subprocess

_DM_BIN = os.getenv("STRATIS_DMSETUP_BIN", "/usr/sbin/dmsetup")


def _get_stratis_devices():
    """
    Return a list of stratis DM devices
    :return: A list of stratis DM devices
    """
    output = subprocess.check_output([_DM_BIN, "ls"])
    decoded = output.decode("utf-8")
    return [l for l in decoded.split() if l.startswith("stratis-")]


def _remove_device(device):
    """
    Attempt to remove a device mapper device.
    :param device: The device to remove
    :return: True if device was removed, else False
    """
    return_code = subprocess.call([_DM_BIN, "remove", device], stderr=subprocess.PIPE)
    return return_code == 0


def remove_stratis_setup():
    """
    Removes all device mapper tables which are related to stratis (best attempt)
    :return: None
    """
    attempts = 0
    devices = _get_stratis_devices()
    max_loops = len(devices) * 5

    # There is some dependency ordering with regards to dm tables, we will
    # simply iterate over the list attempting to remove them until they are
    # all gone or we give up trying.
    while devices != [] and attempts < max_loops:
        attempts += 1

        # Make a copy of the list as we are traversing a list we want to
        # modify.
        for dev in devices.copy():
            if _remove_device(dev):
                devices.remove(dev)


if __name__ == "__main__":
    remove_stratis_setup()
