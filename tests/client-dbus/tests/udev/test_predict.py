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
Test that predictions of space usage match the actual.
"""

# isort: STDLIB
import json
import os
import subprocess

# isort: LOCAL
from stratisd_client_dbus import (
    MOBlockDev,
    MOPool,
    ObjectManager,
    blockdevs,
    get_object,
    pools,
)
from stratisd_client_dbus._constants import TOP_OBJECT

from ._utils import (
    OptionalKeyServiceContextManager,
    ServiceContextManager,
    UdevTest,
    create_pool,
    random_string,
)

_STRATIS_PREDICT_USAGE = os.environ["STRATIS_PREDICT_USAGE"]


def _call_predict_usage(encrypted, sizes):
    """
    Call stratis-predict-usage and return JSON resut.

    :param bool encrypted: true if pool is to be encrypted
    :param sizes: list of sizes of devices for pool
    :type sizes: list of str
    """
    with subprocess.Popen(
        [_STRATIS_PREDICT_USAGE]
        + ["--device-size=%s" % size for size in sizes]
        + (["--encrypted"] if encrypted else []),
        stdout=subprocess.PIPE,
    ) as command:
        outs, _ = command.communicate()
        prediction = json.loads(outs)

    return prediction


def _call_blockdev_size(dev):
    """
    Get the blockdev size for a device in bytes.
    :param str dev: device path
    :rtype: str
    """
    with subprocess.Popen(
        ["blockdev", "--getsize64", dev],
        stdout=subprocess.PIPE,
    ) as command:
        outs, _ = command.communicate()

    return outs.decode().rstrip("\n")


class TestSpaceUsagePrediction(UdevTest):
    """
    Test relations of prediction to reality.
    """

    def _test_prediction(self, pool_name):
        """
        Helper function to verify that the prediction matches the reality to
        an acceptable degree.

        :param str pool_name: the name of the pool to test
        """
        proxy = get_object(TOP_OBJECT)
        managed_objects = ObjectManager.Methods.GetManagedObjects(proxy, {})

        pool_object_path, pool = next(
            pools(props={"Name": pool_name})
            .require_unique_match(True)
            .search(managed_objects)
        )

        modevs = [
            MOBlockDev(info)
            for objpath, info in blockdevs(props={"Pool": pool_object_path}).search(
                managed_objects
            )
        ]

        mopool = MOPool(pool)

        encrypted = mopool.Encrypted()

        block_devices = [modev.PhysicalPath() for modev in modevs]

        sizes = [_call_blockdev_size(dev) for dev in block_devices]

        prediction = _call_predict_usage(encrypted, sizes)

        self.assertEqual(mopool.TotalPhysicalSize(), prediction["total"])

        (success, total_physical_used) = mopool.TotalPhysicalUsed()
        if not success:
            raise RuntimeError("Pool's TotalPhysicalUsed property was invalid.")

        self.assertEqual(total_physical_used, prediction["used"])

    def test_prediction(self):
        """
        Verify that the prediction of space used equals the reality.
        """
        device_tokens = self._lb_mgr.create_devices(4)
        devnodes = self._lb_mgr.device_files(device_tokens)

        with ServiceContextManager():
            pool_name = random_string(5)
            create_pool(pool_name, devnodes)
            self.wait_for_pools(1)
            self._test_prediction(pool_name)

    def test_prediction_encrypted(self):
        """
        Verify that the prediction of space used equals the reality if pool
        is encrypted.
        """
        device_tokens = self._lb_mgr.create_devices(4)
        devnodes = self._lb_mgr.device_files(device_tokens)

        (key_description, key) = ("key_spec", "data")
        with OptionalKeyServiceContextManager(key_spec=[(key_description, key)]):
            pool_name = random_string(5)
            create_pool(pool_name, devnodes, key_description=key_description)
            self.wait_for_pools(1)
            self._test_prediction(pool_name)
