#!/usr/bin/python3
#
# Copyright 2018 Red Hat, Inc.
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
check cargo dependencies' versions
"""


# isort: STDLIB
#!/usr/bin/python
import pprint
import re
import subprocess
import sys

# isort: THIRDPARTY
import requests


def main():
    """
    The main method
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

    while True:

        line = proc.stdout.readline()

        if not line:
            break

        # Convert byte object into string
        line_str = line.decode("utf-8")

        # Extract dependencies, versions, and platforms and fill in data structure line by line
        line_split = line_str.split()

        temp = line_split[0]
        temp_split = temp.split("->")
        dependency = temp_split[0]
        version = line_split[1]
        platform = line_split[5]

        # If the dependency is windows-specific, ignore it
        if "windows" in platform:
            continue

        # If dependency is pulled in by another dependency, extract what the dependency it's pulled
        # in by and add dict entry
        if len(temp_split) > 1:
            pulled_in_by = temp_split[1]
            cargo_outdated_output[dependency] = (version, pulled_in_by, platform)

        # Otherwise add dict entry with None
        else:
            cargo_outdated_output[dependency] = (version, None, platform)

    # DEBUGGING
    print("NOW PRINTING UNMODIFIED DICT\n")

    print_var = pprint.PrettyPrinter(width=41, compact=True)
    print_var.pprint(cargo_outdated_output)

    # DEBUGGING
    print("\n\nNOW PRINTING MODIFICATIONS TO DICT\n")

    # Remove keys such that the dependency is pulled in by a windows-specific dependency
    for key in cargo_outdated_output:
        new_key = cargo_outdated_output[key][1]
        if new_key is not None:
            if new_key in cargo_outdated_output:
                # needs regex
                if "windows" in cargo_outdated_output[new_key][2]:
                    cargo_outdated_output.pop(new_key)

                    # DEBUGGING
                    print(
                        "removed "
                        + new_key
                        + "because"
                        + new_key
                        + "'s platform is "
                        + cargo_outdated_output[new_key][2]
                    )
                else:
                    print(
                        "did NOT remove "
                        + new_key
                        + "because"
                        + new_key
                        + "'s platform is "
                        + cargo_outdated_output[new_key][2]
                    )

    # DEBUGGING
    print("\n\nNOW PRINTING MODIFIED DICT\n")

    print_var.pprint(cargo_outdated_output)

    # Lists that categorized dependencies will be placed in
    outdated = []
    not_outdated = []
    not_found = []

    # Populate with dependency -> version
    koji_dict = {}

    # Check dict contents against Koji packages list
    requests_var = requests.get(
        "https://kojipkgs.fedoraproject.org/repos/rawhide/latest/x86_64/pkglist"
    )
    packages = requests_var.text

    my_reg_ex = re.compile("^toplink\/packages\/(rust-)?([^\/]*)\/([^\/]*)\/[^]*)]*")

    koji_dict = {
        my_reg_ex.match(line)[i][0]: my_reg_ex.match(line)[i][1]
        for i in packages.splitlines()
    }

    # DEBUGGING
    print("\n\nNOW PRINTING KOJI DICT\n")
    print_var.pprint(koji_dict)

    # DEBUGGING
    print("\n\nNOW PRINTING KEY RESULTS\n")

    for key in cargo_outdated_output:
        if key == "Name" or key == "----":
            continue

        if key in koji_dict.keys():
            if koji_dict[key] != version:
                print(
                    "    OUTDATED: The current key, "
                    + key
                    + " ~~~~~~~ because of comparison between:   "
                    + koji_dict[key]
                    + " and "
                    + version
                )
                outdated.append(key)
            else:
                print(
                    "NOT OUTDATED: The current key, "
                    + key
                    + " ~~~~~~~ because of comparison between:   "
                    + koji_dict[key]
                    + " and "
                    + version
                )
                not_outdated.append(key)
        else:
            print("   not found: The current key, " + key)
            not_found.append(key)

    print("\n\nRESULTS")

    print("\nThe following packages are outdated:")
    print(outdated)

    print("\nThe following packages are not outdated:")
    print(not_outdated)

    print(
        "\nThe following packages from the 'Cargo Outdated' output were not found in the koji:"
    )
    print(not_found)

    print("\n")


if __name__ == "__main__":
    sys.exit(main())
