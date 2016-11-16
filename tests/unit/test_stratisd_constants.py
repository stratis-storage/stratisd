# Copyright 2016 Red Hat, Inc.
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
Test operation of StratisdConstants class and related classes.
"""


import unittest

from stratisd_client_dbus._stratisd_constants import StratisdConstants

class BuildTestCase(unittest.TestCase):
    """
    Test building the class.
    """

    def testBuild(self):
        """
        Test that building yields a class w/ the correct properties.
        """
        fields = {'FIELD1' : 2, 'FIELD2': 0}
        klass = StratisdConstants.build_class('Test', fields)
        for key, item in fields.items():
            self.assertEqual(getattr(klass, key), item)
