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
# import pprint
import requests


def build_rustc_cfg_dict():
    """
    :returns: dict containing information from the output of `rustc --print cfg`
    :rtype: dict
    """
    command = ["rustc", "--print", "cfg"]
    proc = subprocess.Popen(command, stdout=subprocess.PIPE)

    rustc_cfg_dict = {}

    pattern = r'target_([^\s]*)="([^\s]*)"'
    my_reg_ex = re.compile(pattern)

    while True:
        # splitlines could avoid break
        line_bo_2 = proc.stdout.readline()

        if not line_bo_2:
            break

        line_str = line_bo_2.decode("utf-8")
        matches = my_reg_ex.match(line_str)

        if matches is not None:
            key = "target_" + matches.group(1)
            value = matches.group(2)
            rustc_cfg_dict[key] = value

        elif matches != "debug_assertions":
            rustc_cfg_dict["cfg"] = line_str.rstrip()

    return rustc_cfg_dict


def process_all(all_match, not_re, basic_re, rustc_cfg_dict):
    """
    todo
    """
    all_args = all_match.group(1).split(", ")

    for all_arg in all_args:
        not_match = not_re.match(all_arg)
        if not_match is not None:
            if not process_not(not_match, rustc_cfg_dict):
                return False

        basic_match = basic_re.match(all_arg)
        if basic_match is not None:
            if not process_basic(basic_match, rustc_cfg_dict):
                return False
    return True


def process_any(any_match, not_re, basic_re, rustc_cfg_dict):
    """
    todo
    """
    any_args = any_match.group(1).split(", ")
    for any_arg in any_args:
        not_match = not_re.match(any_arg)
        if not_match is not None:
            if process_not(not_match, rustc_cfg_dict):
                return True

        basic_match = basic_re.match(any_arg)
        if basic_match is not None:
            if process_basic(basic_match, rustc_cfg_dict):
                return True
    return False


def process_basic(basic_match, rustc_cfg_dict):
    """
    todo
    """
    key_value_pattern = r"\(([^-]*) = ([^-]*)\)"
    key_value_re = re.compile(key_value_pattern)
    key_value_match = key_value_re.match(basic_match.group(1))

    if key_value_match is not None:
        return bool(
            rustc_cfg_dict[key_value_match.group(1)] == key_value_match.group(2)
        )

    return bool(rustc_cfg_dict["cfg"] == basic_match.group(1))


def process_not(not_match, rustc_cfg_dict):
    """
    todo
    """
    return not process_basic(not_match, rustc_cfg_dict)


def build_re_dict(to_parse):
    """
    todo
    """

    re_dict = {}

    all_pattern = r"all\(([^-]*)\)"
    any_pattern = r"any\(([^-]*)\)"
    not_pattern = r"not\(([^-]*)\)"
    basic_pattern = r"([^-]*)"

    re_dict["all_re"] = re.compile(all_pattern)
    re_dict["any_re"] = re.compile(any_pattern)
    re_dict["not_re"] = re.compile(not_pattern)
    re_dict["basic_re"] = re.compile(basic_pattern)

    re_dict["all_match"] = re_dict["all_re"].match(to_parse)
    re_dict["any_match"] = re_dict["any_re"].match(to_parse)
    re_dict["not_match"] = re_dict["not_re"].match(to_parse)
    re_dict["basic_match"] = re_dict["basic_re"].match(to_parse)

    return re_dict


def parse_cfg_format_platform(cfg_match, rustc_cfg_dict):
    """
    todo
    """

    re_dict = build_re_dict(cfg_match.group(1))

    if re_dict["all_match"] is not None:
        return process_all(
            re_dict["all_match"],
            re_dict["not_re"],
            re_dict["basic_re"],
            rustc_cfg_dict,
        )

    if re_dict["any_match"] is not None:
        return process_any(
            re_dict["any_match"],
            re_dict["not_re"],
            re_dict["basic_re"],
            rustc_cfg_dict,
        )

    if re_dict["not_match"] is not None:
        return process_not(re_dict["not_match"], rustc_cfg_dict)

    if re_dict["basic_match"] is not None:
        return process_basic(re_dict["basic_match"], rustc_cfg_dict)
    return False


def parse_target_format_platform(unparsed_platform, rustc_cfg_dict):
    """
    todo
    """

    # empty words case - is it supposed to return true?
    if unparsed_platform == "---":
        return True

    target_components = unparsed_platform.split("-")
    return (
        len(target_components) == 4
        and rustc_cfg_dict["target_arch"] == target_components[0]
        and rustc_cfg_dict["target_vendor"] == target_components[1]
        and rustc_cfg_dict["target_os"] == target_components[2]
        and rustc_cfg_dict["target_env"] == target_components[3]
    )


def parse_platform(unparsed_platform):
    """
    :param unparsed_platform: platform to parse
    :type unparsed_platform:
    :returns: duple containing extracted information and whether or not it's a triple
    :rtype: bool
    """

    cfg_pattern = r"cfg\(([^-]*)\)"
    cfg_re = re.compile(cfg_pattern)
    cfg_match = cfg_re.match(unparsed_platform)

    rustc_cfg_dict = build_rustc_cfg_dict()

    if cfg_match is not None:
        return parse_cfg_format_platform(cfg_match, rustc_cfg_dict)

    return parse_target_format_platform(unparsed_platform, rustc_cfg_dict)


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

        print("CURRENTLY PARSING PLATFORM: " + matches.group(6))

        platform = matches.group(6)
        include = parse_platform(platform)

        dependencies = matches.group(1)
        version = matches.group(2)

        if "->" not in dependencies:
            dependency = dependencies
            cargo_outdated_output[dependency] = (version, None, platform, include)
        else:
            dependencies_split = dependencies.split("->")
            pulled_in_by = dependencies_split[0]
            dependency = dependencies_split[1]
            cargo_outdated_output[dependency] = (
                version,
                pulled_in_by,
                platform,
                include,
            )

    # DEBUGGING
    #        print("\n\nNOW PRINTING DICT\n")
    #        print_var = pprint.PrettyPrinter(width=41, compact=True)
    #        print_var.pprint(cargo_outdated_output)

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
    print("\t\tinclude?:\t\tplatform:\t\tkoji:\t\t\tcargo:\t\tdependency:\n")
    # Lists that categorized dependencies will be placed in
    outdated = []
    not_outdated = []
    not_found = []

    for key in cargo_outdated_dict:

        version = cargo_outdated_dict[key][0]
        platform = cargo_outdated_dict[key][2]
        include = cargo_outdated_dict[key][3]
        if key in ("Name", "----"):
            continue

        if key in koji_repo_dict.keys():
            if koji_repo_dict[key] != version:
                print(
                    "    OUTDATED: "
                    + str(include)
                    + "\t\t\t"
                    + platform
                    + "\t\t\t"
                    + koji_repo_dict[key]
                    + "\t\t"
                    + version
                    + "\t\t"
                    + key
                )
                outdated.append(key)
            else:
                print(
                    "NOT OUTDATED: "
                    + str(include)
                    + "\t\t\t"
                    + platform
                    + "\t\t\t"
                    + koji_repo_dict[key]
                    + "\t\t"
                    + version
                    + "\t\t"
                    + key
                )
                not_outdated.append(key)
        else:
            print(
                "   not found: "
                + str(include)
                + "\t\t\t"
                + platform
                + "\t\t\t\t\t\t"
                + key
            )
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
