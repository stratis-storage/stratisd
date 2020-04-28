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

# isort: STDLIB
import os
import subprocess
import tempfile
import uuid

_LOSETUP_BIN = os.getenv("STRATIS_LOSETUP_BIN", "/usr/sbin/losetup")

_SIZE_OF_DEVICE = 1024 ** 4  # 1 TiB

def _check_tokens(self, tokens):
    if not all(token in self.devices for token in tokens):
        raise RuntimeError("No tokens found")

class LoopBackDevices:
    """
    Class for creating and managing loop back devices which are needed for
    specific types of udev event testing.
    """

    def __init__(self):
        """
        Class constructor which creates a temporary directory to store backing
        file in.
        """
        self.dir = tempfile.mkdtemp("_stratis_loop_back")
        self.count = 0
        self.devices = {}

    def create_devices(self, number):
        """
        Create new loop back devices.

        Note: The first time a loop back device is known it will generate
        a udev "add" event, subsequent backing file changes do not, thus we
        will need to generate it synthetically.
        :param int number: the number of devices to create
        :return: list of keys for the devices
        :rtype: list of uuid.UUID
        """
        tokens = []
        for _ in range(number):
            backing_file = os.path.join(self.dir, "block_device_%d" % self.count)
            self.count += 1

            with open(backing_file, "ab") as dev:
                dev.truncate(_SIZE_OF_DEVICE)

            device = str.strip(
                subprocess.check_output(
                    [_LOSETUP_BIN, "-f", "--show", backing_file]
                ).decode("utf-8")
            )

            token = uuid.uuid4()
            self.devices[token] = (device, backing_file)
            tokens.append(token)

        return tokens

    def unplug(self, tokens):
        """
        Remove the devices from the /dev tree, but doesn't remove backing file
        :param tokens: Opaque representation of some loop back devices
        :type tokens: list of uuid.UUID
        :return: None
        :raises: AssertionError if any token not found
        """
        _check_tokens(self, tokens)
        for token in tokens:
            (device, backing_file) = self.devices[token]
            subprocess.check_call([_LOSETUP_BIN, "-d", device])
            self.devices[token] = (None, backing_file)

    def generate_udev_add_events(self, tokens):
        """
        Synthetically create "add" udev event for specified loop back devices
        :param tokens: Opaque representation of some loop back devices
        :type tokens: list of uuid.UUID
        :return: None
        :raises RuntimeError: if any token not found or missing device node
        """
        _check_tokens(self, tokens)
        for token in tokens:
            (device, _) = self.devices[token]

            if device is None:
                raise RuntimeError("Device node is missing")

            device_name = os.path.split(device)[-1]
            ufile = os.path.join("/sys/block", device_name, "uevent")
            with open(ufile, "w") as event:
                event.write("add")

    def hotplug(self, tokens):
        """
        Attaches an existing backing file to specified loop back devices
        :param tokens: Opaque representation of some loop back devices
        :type tokens: list of uuid.UUID
        :return: None
        :raise AssertionError: if token not present
        """
        _check_tokens(self, tokens)
        for token in tokens:
            (_, backing_file) = self.devices[token]

            device = str.strip(
                subprocess.check_output(
                    [_LOSETUP_BIN, "-f", "--show", backing_file]
                ).decode("utf-8")
            )
            self.devices[token] = (device, backing_file)

        self.generate_udev_add_events(tokens)

    def device_files(self, tokens):
        """
        Return the block device full names for a list of tokens
        :param tokens: Opaque representation of some loop back devices
        :type tokens: list of uuid.UUID
        :return: The list devices corresponding to the tokens
        :rtype: list of str
        :raises: KeyError if any token is not found
        :raises: AssertionError if any devnode is None
        """
        result = [self.devices[token][0] for token in tokens]
        assert all([devnode is not None for devnode in result])
        return result

    def destroy_devices(self):
        """
        Detach loopbacks and remove backing files
        :return:
        """
        for (device, backing_file) in self.devices.values():
            if device is not None:
                subprocess.check_call([_LOSETUP_BIN, "-d", device])
            os.remove(backing_file)

        self.devices = {}
        self.count = 0

    def destroy_all(self):
        """
        Detach all the devices and delete the file(s) and directory!
        :return: None
        """
        self.destroy_devices()
        os.rmdir(self.dir)
        self.dir = None
