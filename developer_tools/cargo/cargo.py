#!/usr/bin/python
# isort: STDLIB
import pprint
import re
import subprocess
import sys
from subprocess import PIPE, Popen, run

# isort: THIRDPARTY
import requests
from lxml import etree, html


def main():

    # The versions are stored in a dictionary (for constant lookup).
    # Key type: a string
    # Key represents: a dependency
    # Value type: a list of 3-tuples containing 1) a string, 2) None or a string, and 3) a string
    # Value represents:
    # 1) the 'Project' version of the dependency
    # 2) the dependency the dependency is pulled in by, or none if the dependency is pinned in Cargo.toml
    # 3) the platform
    d = {}

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

        # If dependency is pulled in by another dependency, extract what the dependency it's pulled in by and add dict entry
        if len(temp_split) > 1:
            pulled_in_by = temp_split[1]
            d[dependency] = (version, pulled_in_by, platform)

        # Otherwise add dict entry with None
        else:
            d[dependency] = (version, None, platform)

    # DEBUGGING
    print("NOW PRINTING UNMODIFIED DICT\n")

    pp = pprint.PrettyPrinter(width=41, compact=True)
    pp.pprint(d)

    # DEBUGGING
    print("\n\nNOW PRINTING MODIFICATIONS TO DICT\n")

    # Remove keys such that the dependency is pulled in by a windows-specific dependency
    for key in d:
        new_key = d[key][1]
        if new_key is not None:
            if new_key in d:
                # needs regex
                if "windows" in d[new_key][2]:
                    d.pop(new_key)

                    # DEBUGGING
                    print(
                        "removed "
                        + new_key
                        + "because"
                        + new_key
                        + "'s platform is "
                        + d[new_key][2]
                    )
                else:
                    print(
                        "did NOT remove "
                        + new_key
                        + "because"
                        + new_key
                        + "'s platform is "
                        + d[new_key][2]
                    )

    # DEBUGGING
    print("\n\nNOW PRINTING MODIFIED DICT\n")

    pp = pprint.PrettyPrinter(width=41, compact=True)
    pp.pprint(d)

    # Populate with dependency -> version
    koji_dict = {}

    # Check dict contents against Koji packages list
    r = requests.get(
        "https://kojipkgs.fedoraproject.org/repos/rawhide/latest/x86_64/pkglist"
    )
    packages = r.text

    # NOTE: re.findall call returns [('glibc32', '2.30')]
    for line in packages.splitlines():
        matches = re.findall(
            "^toplink\/packages\/([^\/]*)\/([^\/]*)\/[^]*)]*", packages
        )
        key = matches[0][0]
        value = matches[0][1]
        # print("Got a match: "+ key + value)
        koji_dict[key] = value

    # DEBUGGING
    print("\n\nNOW PRINTING KOJI DICT\n")
    koji_pp = pprint.PrettyPrinter(width=41, compact=True)
    koji_pp.pprint(koji_dict)

    # Lists that categorized dependencies will be placed in
    outdated = []
    not_outdated = []
    not_found = []

    # DEBUGGING
    print("\n\nNOW PRINTING KEY RESULTS\n")

    for key in d:
        if key == "Name" or key == "----":
            continue

        if packages.find("/" + key + "/") != -1:
            start_index = packages.index(key) + len(key) + 1
            end_index = start_index + len(version)
            if (packages[start_index:end_index] != version) or (
                "rust-" + packages[start_index:end_index] != version
            ):
                print(
                    "    OUTDATED: The current key, "
                    + key
                    + " ~~~~~~~ because of comparison between:   "
                    + packages[start_index:end_index]
                    + " and "
                    + version
                )
                outdated.append(key)
            else:
                print(
                    "NOT OUTDATED: The current key, "
                    + key
                    + " ~~~~~~~ because of comparison between:   "
                    + packages[start_index:end_index]
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
