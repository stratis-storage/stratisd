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
Test invariants on tables in implementation.
"""


import unittest

from stratisd_client_dbus import Manager
from stratisd_client_dbus import Pool

class KeysTestCase(unittest.TestCase):
    """
    Test that every map contains all the designated keys.
    """

    def testManager(self):
        """
        Test that Manager's maps are correct.
        """
        methods = frozenset(Manager.MethodNames)
        # pylint: disable=protected-access
        self.assertEqual(methods, frozenset(Manager._INPUT_SIGS.keys()))
        self.assertEqual(methods, frozenset(Manager._XFORMERS.keys()))

    def testPool(self):
        """
        Test that Pool's maps are correct.
        """
        methods = frozenset(Pool.MethodNames)
        # pylint: disable=protected-access
        self.assertEqual(methods, frozenset(Pool._INPUT_SIGS.keys()))
        self.assertEqual(methods, frozenset(Pool._XFORMERS.keys()))
