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
import re
import sys
from getpass import getpass

from github import Github


def main():
    """
    Main function
    """

    if len(sys.argv) < 2:
        print("USAGE: %s <RELEASE_VERSION>" % sys.argv[0])
        print("\tRELEASE_VERSION: MAJOR.MINOR.PATCH")
        raise RuntimeError("One positional argument is required")

    release_version = sys.argv[1]
    if re.match(r"^[0-9]+\.[0-9]+\.[0-9]$", release_version) is None:
        raise RuntimeError("Invalid release version %s provided" % release_version)

    api_key = os.environ.get("GITHUB_API_KEY")
    if api_key is None:
        api_key = getpass("API key: ")

    git = Github(api_key)

    repo = git.get_repo("stratis-storage/stratisd")

    release = repo.create_git_release(
        "v%s" % release_version,
        "Version %s" % release_version,
        "See changelog here: https://github.com/stratis-storage/stratisd/blob/master/CHANGES.txt",
        draft=True,
    )

    label = "stratisd-%s-vendor.tar.gz" % release_version
    release.upload_asset(label, label)


if __name__ == "__main__":
    try:
        main()
    except Exception as err:  # pylint: disable=broad-except
        print(err)
        sys.exit(1)
