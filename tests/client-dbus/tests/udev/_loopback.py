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

    def create_device(self):
        """
        Create a new loop back device, sparse backing file and attaching it.

        Note: The first time a loop back device is known it will generate
        a udev "add" event, subsequent backing file changes do not, thus we
        will need to generate it synthetically.
        :return: opaque handle, done as device representing block device will
                 change.
        """
        backing_file = os.path.join(self.dir, "block_device_%d" % self.count)
        self.count += 1

        with open(backing_file, "ab") as bd:
            bd.truncate(_SIZE_OF_DEVICE)

        device = str.strip(
            subprocess.check_output(
                [_LOSETUP_BIN, "-f", "--show", backing_file]
            ).decode("utf-8")
        )

        token = uuid.uuid4()
        self.devices[token] = (device, backing_file)
        return token

    def unplug(self, token):
        """
        Remove the device from the /dev tree, but doesn't remove backing file
        :param token: Opaque representation of some loop back device
        :return: None
        """
        if token in self.devices:
            (device, backing_file) = self.devices[token]
            subprocess.check_call([_LOSETUP_BIN, "-d", device])
            self.devices[token] = (None, backing_file)

    def generate_udev_add_event(self, token):
        """
        Synthetically create "add" udev event for this loop back device
        :param token: Opaque representation of some loop back device
        :return: None
        """
        if token in self.devices:
            (device, _) = self.devices[token]

            if device is not None:
                device_name = os.path.split(device)[-1]
                ufile = os.path.join("/sys/block", device_name, "uevent")
                with open(ufile, "w") as e:
                    e.write("add")

    def hotplug(self, token):
        """
        Attaches an existing backing file to a loop back device
        :param token: Opaque representation of some loop back device
        :return: None
        """
        if token in self.devices:
            (_, backing_file) = self.devices[token]

            device = str.strip(
                subprocess.check_output(
                    [_LOSETUP_BIN, "-f", "--show", backing_file]
                ).decode("utf-8")
            )
            self.devices[token] = (device, backing_file)

            # Make sure an add occurs
            self.generate_udev_add_event(token)

    def device_file(self, token):
        """
        Return the block device full name for a loopback token
        :param token: Opaque representation of some loop back device
        :return: Full file path or None if not currently attached
        :raises: KeyError if token is unknown
        :raises: AssertionError if devnode is None
        """
        devnode, _ = self.devices[token]
        assert devnode is not None
        return devnode

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
