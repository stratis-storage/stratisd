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
Check cargo dependencies' versions
"""


# isort: STDLIB
#!/usr/bin/python
# import pprint
import re
import subprocess
import sys

# isort: THIRDPARTY
import requests
from platform_parser import parse_platform


def build_cargo_outdated_dict():
    """
    :returns: cargo outdated information
    :rtype: dict
    """
    # The versions are stored in a dictionary (for constant lookup).
    # Key type: a string
    # Key represents: a dependency
    # Value type: a list of 3-tuples containing 1) a string, 2) None or a string, and 3) a string
    # Value represents:
    # 1) the 'Project' version of the dependency
    # 2) the dependency the dependency is pulled in by, or none if the dependency is pinned in
    # Cargo.toml
    # 3) the platform

    cargo_outdated_output = {}

    # Run cargo-outdated
    command = ["cargo", "outdated"]
    proc = subprocess.Popen(command, stdout=subprocess.PIPE)

    pattern = r"([^\s]*)\s*([^\s]*)\s*([^\s]*)\s*([^\s]*)\s*([^\s]*)\s*(.*)"
    my_reg_ex = re.compile(pattern)

    while True:
        line_bo = proc.stdout.readline()

        if not line_bo:
            break

        line_str = line_bo.decode("utf-8")
        matches = my_reg_ex.match(line_str)

        platform = parse_platform(matches.group(6))

        dependencies = matches.group(1)
        version = matches.group(2)

        if "->" not in dependencies:
            dependency = dependencies
            cargo_outdated_output[dependency] = (version, None, platform)
        else:
            dependencies_split = dependencies.split("->")
            pulled_in_by = dependencies_split[0]
            dependency = dependencies_split[1]
            cargo_outdated_output[dependency] = (version, pulled_in_by, platform)

    # DEBUGGING
    #    print("\n\nNOW PRINTING DICT\n")
    #    print_var = pprint.PrettyPrinter(width=41, compact=True)
    #    print_var.pprint(cargo_outdated_output)

    return cargo_outdated_output


def build_koji_repo_dict(cargo_outdated_output):
    """
    :param cargo_outdated_output: cargo outdated information
    :type cargo_outdated_output: dict
    :returns: koji repo information
    :rtype: dict
    """
    # Populate with dependency -> version
    koji_dict = {}

    # Check dict contents against Koji packages list
    requests_var = requests.get(
        "https://kojipkgs.fedoraproject.org/repos/rawhide/latest/x86_64/pkglist"
    )
    packages = requests_var.text

    pattern = r"^toplink/packages/(rust-)?([^\/]*?)/([^\/]*?)/[^]*)]*"
    my_reg_ex = re.compile(pattern)

    for line in packages.splitlines():
        matches = my_reg_ex.match(line)
        if matches.group(2) in cargo_outdated_output.keys():
            koji_dict[matches.group(2)] = matches.group(3)

    # DEBUGGING
    #    print("\n\nNOW PRINTING KOJI DICT\n")
    #    print_var = pprint.PrettyPrinter(width=41, compact=True)
    #    print_var.pprint(koji_dict)

    return koji_dict


def print_results(cargo_outdated_dict, koji_repo_dict):
    """
    :param cargo_outdated_dict: cargo outdated information
    :type cargo_outdated_dict: dict
    :param koji_repo_dict: koji repo information
    :type koji_repo_dict: dict
    """
    # DEBUGGING
    print("\n\nNOW PRINTING KEY RESULTS\n")
    print("\t\tkoji:\t\t\tcargo:\t\t\tdependency:\t\tplatform:\n")
    # Lists that categorized dependencies will be placed in
    outdated = []
    not_outdated = []
    not_found = []

    for key in cargo_outdated_dict:

        version = cargo_outdated_dict[key][0]
        platform = cargo_outdated_dict[key][2]
        if key in ("Name", "----"):
            continue

        if key in koji_repo_dict.keys():
            if koji_repo_dict[key] != version:
                print(
                    "    OUTDATED: "
                    + key
                    + "\t\t\t"
                    + koji_repo_dict[key]
                    + "\t\t\t"
                    + version
                    + "\t\t\t"
                    + platform
                )
                outdated.append(key)
            else:
                print(
                    "NOT OUTDATED: "
                    + key
                    + "\t\t\t"
                    + koji_repo_dict[key]
                    + "\t\t\t"
                    + version
                    + "\t\t\t"
                    + platform
                )
                not_outdated.append(key)
        else:
            print("   not found: " + key + "\t\t\t\t\t\t\t" + platform)
            not_found.append(key)
    print("\n\nRESULTS")

    print(
        "\nThe following packages that were outputted by 'cargo outdated' are outdated"
        + " with respect to the koji repo:"
    )
    print(outdated)

    print(
        "\nThe following packages that were outputted by 'cargo outdated' are not outdated"
        + " with respect to the koji repo:"
    )
    print(not_outdated)

    print(
        "\nThe following packages that were outputted by 'cargo outdated' were not found"
        " in the koji repo:"
    )
    print(not_found)

    print("\n")


def main():
    """
    The main method
    """
    cargo_outdated_dict = build_cargo_outdated_dict()
    koji_repo_dict = build_koji_repo_dict(cargo_outdated_dict)
    print_results(cargo_outdated_dict, koji_repo_dict)


if __name__ == "__main__":
    sys.exit(main())
