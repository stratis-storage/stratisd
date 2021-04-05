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
import subprocess

# isort: LOCAL
from stratisd_client_dbus import MOPool, ObjectManager, get_object, pools
from stratisd_client_dbus._constants import TOP_OBJECT

from ._utils import (
    _STRATIS_PREDICT_USAGE,
    OptionalKeyServiceContextManager,
    ServiceContextManager,
    UdevTest,
    create_pool,
    random_string,
)


class TestSpaceUsagePrediction(UdevTest):
    """
    Test relations of prediction to reality.
    """

    _CAP_DEVICE_STR = "stratis-1-private-%s-physical-originsub"

    def _test_cap_size(self, pool_name, prediction):
        """
        Helper function to verify that the cap device is the correct size.

        :param str pool_name: the name of the pool to test
        :param prediction: JSON output from script
        """
        proxy = get_object(TOP_OBJECT)
        managed_objects = ObjectManager.Methods.GetManagedObjects(proxy, {})

        _pool_object_path, pool = next(
            pools(props={"Name": pool_name})
            .require_unique_match(True)
            .search(managed_objects)
        )

        pool_uuid = MOPool(pool).Uuid()

        cap_name = self._CAP_DEVICE_STR % pool_uuid

        command = subprocess.Popen(
            ["blockdev", "--getsize64", "/dev/mapper/%s" % cap_name],
            stdout=subprocess.PIPE,
            universal_newlines=True,
        )
        cap_device_size, _ = command.communicate()
        self.assertEqual(cap_device_size.rstrip("\n"), prediction["free"])

    def test_prediction(self):
        """
        Verify that the prediction of space used equals the reality.
        """
        device_tokens = self._lb_mgr.create_devices(4)
        devnodes = self._lb_mgr.device_files(device_tokens)
        command = subprocess.Popen(
            [_STRATIS_PREDICT_USAGE] + devnodes, stdout=subprocess.PIPE
        )
        outs, _ = command.communicate()
        prediction = json.loads(outs)

        with ServiceContextManager():
            pool_name = random_string(5)
            create_pool(pool_name, devnodes)
            self.wait_for_pools(1)
            self._test_cap_size(pool_name, prediction)

    def test_prediction_encrypted(self):
        """
        Verify that the prediction of space used equals the reality if pool
        is encrypted.
        """
        device_tokens = self._lb_mgr.create_devices(4)
        devnodes = self._lb_mgr.device_files(device_tokens)
        command = subprocess.Popen(
            [_STRATIS_PREDICT_USAGE, "--encrypted"] + devnodes, stdout=subprocess.PIPE
        )
        outs, _ = command.communicate()
        prediction = json.loads(outs)

        (key_description, key) = ("key_spec", "data")
        with OptionalKeyServiceContextManager(key_spec=[(key_description, key)]):
            pool_name = random_string(5)
            create_pool(pool_name, devnodes, key_description=key_description)
            self.wait_for_pools(1)
            self._test_cap_size(pool_name, prediction)
