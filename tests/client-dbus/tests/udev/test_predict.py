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
import sys

# isort: LOCAL
from stratisd_client_dbus import MOPool, ObjectManager, get_object, pools
from stratisd_client_dbus._constants import TOP_OBJECT

from ._utils import (
    _STRATIS_PREDICT_USAGE,
    ServiceContextManager,
    UdevTest,
    create_pool,
    random_string,
)


class TestSpaceUsagePrediction(UdevTest):
    """
    Test relations of prediction to reality.
    """

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
        print(prediction, file=sys.stderr)

        with ServiceContextManager():
            pool_name = random_string(5)
            create_pool(pool_name, devnodes)
            proxy = get_object(TOP_OBJECT)
            managed_objects = ObjectManager.Methods.GetManagedObjects(proxy, {})

            _pool_object_path, pool = next(
                pools(props={"Name": pool_name})
                .require_unique_match(True)
                .search(managed_objects)
            )

            _pool_uuid = MOPool(pool).Uuid()
