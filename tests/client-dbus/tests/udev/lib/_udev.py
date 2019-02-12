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
Udev Stratis verification
"""
import time

import pyudev

from ._stratis_id import stratis_signature, dump_stratis_signature_area


class StratisBlockDevices:
    """
    Class which handles ensuring that we have the expected state in udev or
    on the actual block devices themselves.
    """

    def __init__(self):
        self._lib_blk_id = False
        self._context = pyudev.Context()

    def expected(self, expected_paths):
        """
        Check that the expected number of stratis devices exist.  If not keep
        checking until they do show up or our timeout has been exceeded.
        :param expected_paths: List of expected device nodes with stratis signatures
        :return: None (May assert)
        """
        num_expected = len(expected_paths)
        found = 0
        start = time.time()
        end_time = start + 10

        while self._lib_blk_id and time.time() < end_time:
            found = sum(1 for _ in self._context.list_devices(
                subsystem='block', ID_FS_TYPE='stratis'))
            if found == num_expected:
                break
            time.sleep(1)

        # If we are not matching our expectations, we may be running on a box
        # that doesn't have blkid support, so lets probe the disks instead.  If
        # we find a stratis disk now, we will set the flag UdevAdd.lib_blk_id to
        # false so we don't waste so much time checking the udev db.
        if found != num_expected and found == 0:
            for blk_dev in self._context.list_devices(subsystem='block'):
                if "DEVNAME" in blk_dev:
                    if stratis_signature(blk_dev["DEVNAME"]):
                        self._lib_blk_id = False
                        found += 1

        if found != num_expected:
            self.dump_state(expected_paths)

        assert found == num_expected

    def dump_state(self, expected_paths):
        """
        Dump everything we can when we are missing stratis devices!
        :param expected_paths: list of devices which we know should have
               signatures
        :return: None
        """
        print("We expect Stratis signatures on %d device(s)" %
              len(expected_paths))
        for d in expected_paths:
            signature = stratis_signature(d)
            print("%s shows signature check of %s" % (d, signature))

            if signature is None:
                # We are really expecting this device to have the signature
                # lets dump the signature area of the disk
                dump_stratis_signature_area(d)

        print("Udev db dump of all block devices")
        for d in self._context.list_devices(subsystem='block'):
            for k, v in d.items():
                print("%s:%s" % (k, str(v)))
            print("")
