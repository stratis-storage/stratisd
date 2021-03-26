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
Calculates information comparing the versions of dependencies in a Rust project
to the versions of dependencies available on Fedora Rawhide.
"""


# isort: STDLIB
import argparse
import json
import re
import subprocess
import sys

# isort: THIRDPARTY
import requests
from semantic_version import Version

KOJI_RE = re.compile(
    r"^toplink/packages/rust-(?P<name>[^\/]*?)/(?P<version>[^\/]*?)/[^]*)]*"
)
VERSION_RE = re.compile(
    r"^\^(?P<major>[0-9]+)(\.(?P<minor>[0-9]+))?(\.(?P<patch>[0-9]+))?$"
)


def _build_koji_repo_dict(crates):
    """
    :param crates: a set of crates
    :type cargo_tree: set of str
    :returns: a dictionary containing information from the koji repo webpage
    the keys are the string representations of dependencies
    the values are the versions of dependencies
    :rtype: dict of str * Version
    """
    koji_repo_dict = {}

    requests_var = requests.get(
        "https://kojipkgs.fedoraproject.org/repos/rawhide/latest/x86_64/pkglist"
    )
    packages = requests_var.text

    for line in packages.splitlines():
        matches = KOJI_RE.match(line)
        if matches is None:
            continue
        name = matches.group("name")
        if name in crates:
            # Fedora appears to be using non-SemVer standard version strings:
            # the standard seems to be to use a "~" instead of a "-" in some
            # places. See https://semver.org/ for the canonical grammar that
            # the semantic_version library adheres to.
            version = matches.group("version").replace("~", "-")
            koji_repo_dict[name] = Version(version)

    # Post-condition: koji_repo_dict.keys() <= cargo_tree.keys().
    # cargo tree may show internal dependencies that are not separate packages
    return koji_repo_dict


def _build_cargo_metadata():
    """
    Build a dict mapping crate to version spec from Cargo.toml.
    """
    command = ["cargo", "metadata", "--format-version=1", "--no-deps"]
    proc = subprocess.Popen(command, stdout=subprocess.PIPE)
    stream = proc.stdout
    metadata_str = stream.readline()
    metadata = json.loads(metadata_str)
    packages = metadata["packages"]
    assert len(packages) == 1
    package = packages[0]
    assert package["name"] == "libstratis"
    dependencies = package["dependencies"]

    result = dict()
    for item in dependencies:
        matches = VERSION_RE.match(item["req"])
        major = int(matches["major"] or 0)
        minor = int(matches["minor"] or 0)
        patch = int(matches["patch"] or 0)
        result[item["name"]] = Version(major=major, minor=minor, patch=patch)

    return result


def main():
    """
    The main method
    """
    parser = argparse.ArgumentParser(
        description=(
            "Compares versions of direct dependencies in Fedora with versions "
            "specified in Cargo.toml. Prints some information to stdout. Rules "
            "for exit code: "
            "if exit code & 0x4 == 0x4, a dependency is missing in "
            "Fedora, if exit code & 0x8 = 0x8, a dependency is higher than "
            "that available in Fedora, if exit code & 0x10 == 0x10, then a "
            "dependency can be bumped to a higher version in Fedora."
        )
    )
    parser.parse_args()

    # Read the dependency versions specified in Cargo.toml
    explicit_dependencies = _build_cargo_metadata()

    # Build koji dict
    koji_repo_dict = _build_koji_repo_dict(frozenset(explicit_dependencies.keys()))

    exit_code = 0x0

    for crate, version in explicit_dependencies.items():
        koji_version = koji_repo_dict.get(crate)
        if koji_version is None:
            print("No Fedora package for crate %s found" % crate, file=sys.stdout)
            exit_code |= 0x4
            continue

        if koji_version < version:
            print(
                "Version %s of crate %s higher than maximum %s that is available on Fedora"
                % (version, crate, koji_version),
                file=sys.stdout,
            )
            exit_code |= 0x8
            continue

        exclusive_upper_bound = (
            version.next_major() if version.major != 0 else version.next_minor()
        )

        if koji_version >= exclusive_upper_bound:
            print(
                "Version %s of crate %s is available in Fedora. Requires update in Cargo.toml"
                % (koji_version, crate),
                file=sys.stdout,
            )
            exit_code |= 0x10

    return exit_code


if __name__ == "__main__":
    sys.exit(main())
