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
Wrapper around nbd block devices
"""

import os
import subprocess
import time

from ._device_backed_file import DeviceFile
from ._utils import rs

_NBDCLIENT_BIN = os.getenv('STATIS_NBDCLIENT_BIN', "/usr/sbin/nbd-client")
_NBDKIT_BIN = os.getenv('STATIS_NBDKIT_BIN', "/usr/sbin/nbdkit")
_NBD_PORT_START = int(os.getenv('STATIS_NBDKIT_PORTSTART', "12000"))


def _kernel_module_present():
    """
    Check for nbd kernel module being loaded.
    :return:
    """
    with open("/proc/modules", "r") as f:
        return len([x for x in f.readlines() if x.startswith("nbd ")]) == 1


def _udev_rule_present():
    """
    Make sure we have the udev rule for nbd block devices.
    :return:
    """
    with open("/usr/lib/udev/rules.d/60-block.rules", "r") as f:
        return len([x for x in f.readlines() if "|nbd*" in x]) == 1


def flight_check():
    """
    Make sure everything is in place for nbd devices to work with Stratis
    :return: None, will assert if anything is missing
    """
    # Make sure module is loaded
    assert _kernel_module_present()

    # Make sure udev rules knows about nbd devices
    assert _udev_rule_present()

    # Make sure we have nbd and nbd-kit executables
    assert os.path.isfile(_NBDCLIENT_BIN)
    assert os.path.isfile(_NBDKIT_BIN)


class NbdDevices(DeviceFile):
    """
    Class for creating and managing nbd devices which are needed for
    specific types of udev event testing.
    """

    def __init__(self):
        super(NbdDevices, self).__init__()
        self.look_up = {}

    @staticmethod
    def _find_first_available():
        """
        Find the first available /dev/nbd<N> that is available
        :return: Tuple (N, dev_node) with first available device node, else
                 (-1, None)
        """
        for i in range(16):  # Default number is 16 [0..15]
            dev_node = os.path.join("/dev", "nbd%d" % i)
            if os.path.exists(dev_node):
                try:
                    subprocess.check_output([_NBDCLIENT_BIN, '-c', dev_node])
                except subprocess.CalledProcessError as e:
                    if e.returncode == 1:
                        return i, dev_node
            else:
                break
        return -1, None

    def attach(self, file):
        """
        Attach a file to a device node
        :param file: File to use as block device
        :return: Device node
        """

        # Attaching a file requires some steps
        # 0. Find an empty device file
        # 1. Start up the server for the file
        # 2. Use the nbd client to connect to nbd device node

        # Find a nbd device node that isn't in use.
        num, dev_node = NbdDevices._find_first_available()
        if num >= 0:
            # We need a different port for each block server

            export_name = rs(5)
            port = _NBD_PORT_START + num
            self.look_up[dev_node] = subprocess.Popen([
                _NBDKIT_BIN, 'file',
                'file=%s' % file, '--port',
                str(port), '--no-fork', '--exportname', export_name
            ])

            # The nbdkit service may not be ready before we try to attach the
            # client, we will retry if we fail.
            for _ in range(3):
                try:
                    subprocess.check_call([
                        _NBDCLIENT_BIN, '-b', '512', '--name', export_name,
                        "localhost",
                        str(port), dev_node
                    ])
                    break
                except subprocess.CalledProcessError:
                    time.sleep(2)
            else:
                # Unable to connect client to service.
                raise AssertionError(
                    "Unable to connect client to block service!")

            return dev_node

        raise Exception("No free device node available!")

    def detach(self, device):
        """
        Detach a device.  Nbd block devices have the desired property where
        you can detach them while they are still in use and thus simulate a
        usb jump drive getting yanked, at least for now anyway.
        :param device: Device node to detach
        :return: None
        """
        if device in self.look_up:
            subprocess.check_call([_NBDCLIENT_BIN, '-c', device])

            # Detach the block device from server
            subprocess.check_call([_NBDCLIENT_BIN, '-d', device])

            process = self.look_up[device]
            process.terminate()
            process.wait()
            del self.look_up[device]
