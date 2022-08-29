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
General constants.
"""

SERVICE = "org.storage.stratis3"
TOP_OBJECT = "/org/storage/stratis3"

REVISION_NUMBER = 4

REVISION = f"r{REVISION_NUMBER}"

BLOCKDEV_INTERFACE = f"org.storage.stratis3.blockdev.{REVISION}"
FETCH_PROPERTIES_INTERFACE = f"org.storage.stratis3.FetchProperties.{REVISION}"
FILESYSTEM_INTERFACE = f"org.storage.stratis3.filesystem.{REVISION}"
MANAGER_INTERFACE = f"org.storage.stratis3.Manager.{REVISION}"
POOL_INTERFACE = f"org.storage.stratis3.pool.{REVISION}"
REPORT_INTERFACE = f"org.storage.stratis3.Report.{REVISION}"
