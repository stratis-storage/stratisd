#!/usr/bin/python3
#
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
Uploads the vendored dependency tarball as a release asset.
"""

import os
import sys

from github import Github


def main():
    """
    Main function
    """
    api_key = os.environ.get("GITHUB_API_KEY")
    if api_key is None:
        raise RuntimeError("GITHUB_API_KEY environment variable is required")

    git = Github(api_key)
    repo = git.get_repo("stratis-storage/stratisd")
    release = repo.get_latest_release()
    tag_name = release.tag_name
    release.upload_asset(
        "stratisd-vendor.tar.gz", "stratisd-vendor-%s.tar.gz" % tag_name
    )


if __name__ == "__main__":
    try:
        main()
    except Exception as err:  # pylint: disable=broad-except
        print(err)
        sys.exit(1)
