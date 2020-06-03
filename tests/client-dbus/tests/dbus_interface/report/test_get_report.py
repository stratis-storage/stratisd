# Copyright 2020 Red Hat, Inc.
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
Test the reporting interface in stratisd.
"""

# isort: STDLIB
import json

# isort: LOCAL
from stratisd_client_dbus import ReportR1, get_object
from stratisd_client_dbus._constants import TOP_OBJECT

from .._misc import SimTestCase, device_name_list

_DEVICE_STRATEGY = device_name_list(1)


class GetReportTestCase(SimTestCase):
    """
    Test that getting valid reports succeeds and that getting invalid reports
    fails.
    """

    _POOLNAME = "reportpool"

    def setUp(self):
        """
        Start stratisd.
        """
        super().setUp()
        self._proxy = get_object(TOP_OBJECT)

    def test_valid_report_name(self):
        """
        Test that errored_pool_report returns a valid JSON response.
        """
        (json_str, return_code, _) = ReportR1.Methods.GetReport(
            self._proxy, {"name": "errored_pool_report"}
        )
        self.assertEqual(return_code, 0)
        # Test that JSON is valid - will raise ValueError if not
        json.loads(json_str)

    def test_invalid_report_name(self):
        """
        Test that an invalid report name returns an error.
        """
        (json_str, return_code, _) = ReportR1.Methods.GetReport(
            self._proxy, {"name": "nonexistent_report"}
        )
        self.assertEqual(json_str, "")
        self.assertNotEqual(return_code, 0)
