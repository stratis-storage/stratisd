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

from hypothesis import given
from hypothesis import settings
from hypothesis import strategies

from stratisd_client_dbus._stratisd_constants import StratisdConstants

class BuildTestCase(unittest.TestCase):
    """
    Test building the class.
    """

    @given(
       strategies.dictionaries(
          strategies.text(strategies.characters(), min_size=1),
          strategies.integers(min_value=0)
       )
    )
    @settings(max_examples=50)
    def testBuild(self, fields):
        """
        Test that building yields a class w/ the correct properties.
        """
        klass = StratisdConstants.build_class('Test', fields)
        for key, item in fields.items():
            self.assertEqual(getattr(klass, key), item)


class GetTestCase(unittest.TestCase):
    """
    Test that getting the class yields a properly formed class.
    """

    @given(
       strategies.lists(
          elements=strategies.tuples(
             strategies.text(strategies.characters(), min_size=1, max_size=10),
             strategies.integers(min_value=0),
             strategies.text(strategies.characters(), max_size=50)
          ),
          average_size=10,
          max_size=30,
          unique_by=lambda x: x[0]
       )
    )
    @settings(max_examples=20)
    def testGet(self, constant_list):
        """
        Verify that the class has the properly set up fields.
        """
        klass = StratisdConstants.get_class("StratisdErrors", constant_list)
        for (x, y, _) in constant_list:
            self.assertEqual(getattr(klass, x), y)
