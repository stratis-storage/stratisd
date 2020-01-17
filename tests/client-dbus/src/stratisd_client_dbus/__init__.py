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

from ._implementation import Blockdev
from ._implementation import FetchProperties
from ._implementation import Filesystem
from ._implementation import Manager
from ._implementation import ManagerR1
from ._implementation import ObjectManager
from ._implementation import Pool
from ._implementation import PoolR1
from ._implementation import blockdevs
from ._implementation import pools
from ._implementation import filesystems
from ._implementation import MOBlockDev
from ._implementation import MOPool

from ._stratisd_constants import StratisdErrors
