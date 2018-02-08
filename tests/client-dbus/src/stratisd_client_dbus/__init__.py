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

from ._implementation import Filesystem
from ._implementation import Manager
from ._implementation import ObjectManager
from ._implementation import Pool

from ._managedobjects import blockdevs
from ._managedobjects import pools
from ._managedobjects import filesystems
from ._managedobjects import MOBlockDev
from ._managedobjects import MOPool

from ._stratisd_constants import StratisdErrors
