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
Representing stratisd contants.
"""

# isort: STDLIB
from enum import IntEnum


class StratisdErrors(IntEnum):
    """
    Stratisd Errors
    """

    OK = 0
    ERROR = 1

    ALREADY_EXISTS = 2
    BUSY = 3
    INTERNAL_ERROR = 4
    NOT_FOUND = 5
