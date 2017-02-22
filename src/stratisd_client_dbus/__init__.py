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
Top-level classes and methods.
"""

from ._connection import get_object

from ._implementation import GMOPool
from ._implementation import Filesystem
from ._implementation import Manager
from ._implementation import Pool

from ._managedobjects import get_managed_objects

from ._stratisd_constants import StratisdErrorsGen
from ._stratisd_constants import StratisdRaidGen
