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

# isort: THIRDPARTY
from justbytes import GiB, Range, TiB

# isort: LOCAL
from stratisd_client_dbus import (
    MOBlockDev,
    MOPool,
    ObjectManager,
    Pool,
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
# Filesystem size limit should be no more than three times actual size
_FILESYSTEM_MULT_LIMIT = 3


def _call_predict_usage(encrypted, device_sizes, *, fs_specs=None, overprovision=True):
    """
    Call stratis-predict-usage and return JSON resut.

    :param bool encrypted: true if pool is to be encrypted
    :param device_sizes: list of sizes of devices for pool
    :type device_sizes: list of str
    :param fs_specs: list of filesystem specs
    :type fs_specs: list of str * Range
    :param bool overprovision: whether it is allowed to overprovision the pool
    """
    with subprocess.Popen(
        [_STRATIS_PREDICT_USAGE, "pool"]
        + ["--device-size=%s" % size for size in device_sizes]
        + (
            []
            if fs_specs is None
            else [("--filesystem-size=%s" % size.magnitude) for _, size in fs_specs]
        )
        + (["--encrypted"] if encrypted else [])
        + ([] if overprovision else ["--no-overprovision"]),
        stdout=subprocess.PIPE,
    ) as command:
        outs, errs = command.communicate()
        if command.returncode != 0:
            raise RuntimeError(
                "Invocation of %s returned an error: %s, %s"
                % (_STRATIS_PREDICT_USAGE, command.returncode, errs)
            )
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


def _possibly_add_filesystems(pool_object_path, *, fs_specs=None):
    """
    Add filesystems to the already created pool to set up testing, if
    filesystms have been specified.

    :param str pool_object_path: the D-Bus object path
    :param fs_specs: the filesystem specs
    :type fs_specs: list of str * Range or NoneType

    :returns: the change in size of pool's TotalPhysicalUsed value
    :rtype: Range
    """
    if fs_specs is not None:
        pool_proxy = get_object(pool_object_path)

        (real, pool_used_pre) = Pool.Properties.TotalPhysicalUsed.Get(pool_proxy)
        if not real:
            raise RuntimeError("Failed to get pool usage before creating filesystems.")

        (_, return_code, message,) = Pool.Methods.CreateFilesystems(
            pool_proxy,
            {"specs": map(lambda x: (x[0], (True, str(x[1].magnitude))), fs_specs)},
        )

        if return_code != 0:
            raise RuntimeError("Failed to create a requested filesystem: %s" % message)

        (real, pool_used_post) = Pool.Properties.TotalPhysicalUsed.Get(pool_proxy)
        if not real:
            raise RuntimeError("Failed to get pool usage after creating filesystems.")

        return Range(pool_used_post) - Range(pool_used_pre)

    return Range(0)


def _get_block_device_sizes(pool_object_path, managed_objects):
    """
    Get sizes of block devices.

    :param pool_object_path: The object path of the designated pool.
    :param managed_objects: managed objects dict
    """
    modevs = [
        MOBlockDev(info)
        for objpath, info in blockdevs(props={"Pool": pool_object_path}).search(
            managed_objects
        )
    ]

    block_devices = [modev.PhysicalPath() for modev in modevs]

    return [_call_blockdev_size(dev) for dev in block_devices]


class TestSpaceUsagePrediction(UdevTest):
    """
    Test relations of prediction to reality.
    """

    def _check_fs_prediction(
        self, pre_prediction, post_prediction, change, *, overprovision=True
    ):
        """
        Check the prediction for filesystems by comparing a prediction
        before filesystems are added to one after filesystems are added.

        It should be greater or equal to the actual change but no greater than
        a statical limit times the actual change.

        :param str pre_prediction: prediction before filesystems
        :param str post_prediction: prediction after filesystems
        :param Range change: the real change in TotalPhysicalUsed result
        :param bool overprovision: whether overprovisioning is allowed on pool
        """
        pre_used = Range(pre_prediction["used"])
        post_used = Range(post_prediction["used"])

        prediction_change = post_used - pre_used

        if overprovision:
            self.assertGreaterEqual(prediction_change, change)
            self.assertLessEqual(prediction_change, _FILESYSTEM_MULT_LIMIT * change)
        else:
            # If no overprovisioning is allowed, the prediction will exceed the
            # actual used for any filesystem.
            self.assertLess(change, prediction_change)

    def _check_prediction(self, prediction, mopool):
        """
        Check the prediction against the values obtained from the D-Bus.

        :param str prediction: result of calling script, JSON format
        :param MOPool mopool: object with pool properties
        """
        encrypted = mopool.Encrypted()

        (success, total_physical_used) = mopool.TotalPhysicalUsed()
        if not success:
            raise RuntimeError("Pool's TotalPhysicalUsed property was invalid.")

        (used_prediction, total_prediction) = (
            prediction["used"],
            prediction["total"],
        )

        if encrypted:
            self.assertLess(mopool.TotalPhysicalSize(), total_prediction)
            self.assertLess(total_physical_used, used_prediction)

            diff1 = Range(total_prediction) - Range(mopool.TotalPhysicalSize())
            diff2 = Range(used_prediction) - Range(total_physical_used)

            self.assertEqual(diff1, diff2)
        else:
            self.assertEqual(mopool.TotalPhysicalSize(), total_prediction)
            self.assertEqual(total_physical_used, used_prediction)

    def _test_prediction(self, pool_name, *, fs_specs=None, overprovision=True):
        """
        Helper function to verify that the prediction matches the reality to
        an acceptable degree.

        :param str pool_name: the name of the pool to test
        :param fs_specs: filesystems to create and test
        :type fs_specs: list of of str * Range or NoneType
        :param bool overprovision: True if overprovisioning is allowed
        """
        proxy = get_object(TOP_OBJECT)

        managed_objects = ObjectManager.Methods.GetManagedObjects(proxy, {})

        pool_object_path, pool = next(
            pools(props={"Name": pool_name})
            .require_unique_match(True)
            .search(managed_objects)
        )

        mopool = MOPool(pool)

        physical_sizes = _get_block_device_sizes(pool_object_path, managed_objects)
        pre_prediction = _call_predict_usage(
            mopool.Encrypted(), physical_sizes, overprovision=overprovision
        )

        self._check_prediction(pre_prediction, mopool)

        change = _possibly_add_filesystems(pool_object_path, fs_specs=fs_specs)

        post_prediction = _call_predict_usage(
            mopool.Encrypted(),
            physical_sizes,
            fs_specs=fs_specs,
            overprovision=overprovision,
        )

        self._check_fs_prediction(
            pre_prediction, post_prediction, change, overprovision=overprovision
        )

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

    def test_prediction_filesystems(self):
        """
        Verify that the prediction of space used is within acceptable limits
        when creating filesystems.
        """
        device_tokens = self._lb_mgr.create_devices(4)
        devnodes = self._lb_mgr.device_files(device_tokens)

        with ServiceContextManager():
            pool_name = random_string(5)
            create_pool(pool_name, devnodes)
            self.wait_for_pools(1)
            self._test_prediction(pool_name, fs_specs=[("fs1", Range(1, TiB))])

    def test_prediction_no_overprov(self):
        """
        Verify that the prediction of space used is within acceptable limits
        when no overprovisioning is allowed.
        """
        device_tokens = self._lb_mgr.create_devices(4)
        devnodes = self._lb_mgr.device_files(device_tokens)

        with ServiceContextManager():
            pool_name = random_string(5)
            create_pool(pool_name, devnodes, overprovision=False)
            self.wait_for_pools(1)
            self._test_prediction(
                pool_name, fs_specs=[("fs1", Range(2, GiB))], overprovision=False
            )
