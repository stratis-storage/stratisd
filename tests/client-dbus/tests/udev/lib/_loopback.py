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
Class to handle loop back devices.
"""

import os
import subprocess

from ._device_backed_file import DeviceFile

_LOSETUP_BIN = os.getenv('STRATIS_LOSETUP_BIN', "/usr/sbin/losetup")


class LoopBackDevices(DeviceFile):
    """
    Class for creating and managing loop back devices which are needed for
    specific types of udev event testing.
    """

    def attach(self, file):
        """
        Attach a file to a device node
        :param file: File to use as block device
        :return: Device node
        """
        result = subprocess.check_output([_LOSETUP_BIN, '-f', '--show', file])
        device = str.strip(result.decode("utf-8"))
        return device

    def detach(self, device):
        """
        Detach a device
        :param device: Device node to detach
        :return: None
        """
        subprocess.check_call([_LOSETUP_BIN, '-d', device])
