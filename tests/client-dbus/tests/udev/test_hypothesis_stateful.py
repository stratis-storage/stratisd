# Copyright 2024 Red Hat, Inc.
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
Use hypothesis stateful testing to test stratisd.
"""

# isort: STDLIB
from time import sleep

# isort: THIRDPARTY
from hypothesis import HealthCheck, settings
from hypothesis.stateful import RuleBasedStateMachine, precondition, rule

from ._utils import _Service, processes


class StratisOrders(RuleBasedStateMachine):
    """
    Rule based machine for doing testing.
    """

    def __init__(self):  # pylint: disable=super-init-not-called
        """
        Initialize.
        """
        super().__init__()
        self.service = _Service()

    @precondition(lambda self: next(processes("stratisd"), None) is not None)
    @rule()
    def start_stratisd(self):
        """
        Start stratisd.
        """
        sleep(5)
        self.service.start_service()

    @precondition(lambda self: True)
    @rule()
    def stop_stratisd(self):
        """
        Stop stratisd.
        """
        sleep(5)
        self.service.stop_service()

    @precondition(lambda self: True)
    @rule()
    def create_pool(self):
        """
        Create a pool.
        """

    @precondition(lambda self: True)
    @rule()
    def destroy_pool(self):
        """
        Destroy a pool.
        """


StratisOrders.TestCase.settings = settings(suppress_health_check=[HealthCheck.too_slow])
TestStratis = StratisOrders.TestCase
