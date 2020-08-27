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
This script leverages cargo-outdated to generate information about Rust
dependencies' outdatedness status with respect to the Koji package list.
"""


# isort: STDLIB
import argparse
import re
import subprocess
import sys

# isort: THIRDPARTY
import requests


def build_rustc_cfg_dict():
    """
    :returns: dict containing information from the output of `rustc --print cfg`
    the keys are the string representations of compilation environment identifers
    the values are the string representations of the allowed compilation environment
    identifer values
    :rtype: dict
    """
    command = ["rustc", "--print", "cfg"]
    proc = subprocess.Popen(command, stdout=subprocess.PIPE)

    rustc_cfg_dict = {}

    rustc_cfg_pattern = r'target_([^\s]*)="([^\s]*)"'
    rustc_cfg_re = re.compile(rustc_cfg_pattern)

    while True:
        line = proc.stdout.readline()

        if line == b"":
            break

        line_str = line.decode("utf-8")
        rustc_cfg_match = rustc_cfg_re.match(line_str)

        if rustc_cfg_match is not None:
            key = "target_" + rustc_cfg_match.group(1)
            value = rustc_cfg_match.group(2)
            rustc_cfg_dict[key] = value

        elif rustc_cfg_match != "debug_assertions":
            rustc_cfg_dict["cfg"] = line_str.rstrip()

    return rustc_cfg_dict


def process_all(all_match, all_re, any_re, not_re, rustc_cfg_dict):
    """
    :param all_match: a match to the compiled "all" regular expression
    :type all_match: re.Match
    :param any_re: the compiled "any" regular expression
    :type any_re: re.Pattern
    :param not_re: the compiled "not" regular expression
    :type not_re: re.Pattern
    :param rustc_cfg_dict: dict containing information from the output of
    `rustc --print cfg`
    :type rustc_cfg_dict: dict
    :returns: a bool indicating whether or not this "all" argument should
    be included based on the contents of rustc_cfg_dict
    :rtype: bool
    """
    all_args = all_match.group(1).split(", ")

    for all_arg in all_args:
        all_match = all_re.match(all_arg)
        any_match = any_re.match(all_arg)
        not_match = not_re.match(all_arg)

        if all_match is not None:
            if not process_all(all_match, all_re, any_re, not_re, rustc_cfg_dict):
                return False

        elif any_match is not None:
            if not process_any(any_match, all_re, any_re, not_re, rustc_cfg_dict):
                return False

        elif not_match is not None:
            if not process_not(not_match, all_re, any_re, not_re, rustc_cfg_dict):
                return False

        elif not process_basic(all_arg, rustc_cfg_dict):
            return False

    return True


def process_any(any_match, all_re, any_re, not_re, rustc_cfg_dict):
    """
    :param any_match: a match to the compiled "any" regular expression
    :type any_match: re.Match
    :param all_re: the compiled "all" regular expression
    :type all_re: re.Pattern
    :param any_re: the compiled "any" regular expression
    :type any_re: re.Pattern
    :param not_re: the compiled "not" regular expression
    :type not_re: re.Pattern
    :param rustc_cfg_dict: dict containing information from the output of
    `rustc --print cfg`
    :type rustc_cfg_dict: dict
    :returns: a bool indicating whether or not this "any" argument should
    be included based on the contents of rustc_cfg_dict
    :rtype: bool
    """
    any_args = any_match.group(1).split(", ")
    for any_arg in any_args:
        all_match = all_re.match(any_arg)
        any_match = any_re.match(any_arg)
        not_match = not_re.match(any_arg)

        if all_match is not None:
            if not process_all(all_match, all_re, any_re, not_re, rustc_cfg_dict):
                return True

        elif any_match is not None:
            if not process_any(any_match, all_re, any_re, not_re, rustc_cfg_dict):
                return True

        elif not_match is not None:
            if not process_not(not_match, all_re, any_re, not_re, rustc_cfg_dict):
                return True

        elif process_basic(any_arg, rustc_cfg_dict):
            return True

    return False


def process_not(not_match, all_re, any_re, not_re, rustc_cfg_dict):
    """
    :param not_match: a match to the compiled "not" regular expression
    :type not_match: re.Match
    :param all_re: the compiled "all" regular expression
    :type all_re: re.Pattern
    :param any_re: the compiled "any" regular expression
    :type any_re: re.Pattern
    :param not_re: the compiled "not" regular expression
    :type not_re: re.Pattern
    :param rustc_cfg_dict: dict containing information from the output of
    `rustc --print cfg`
    :type rustc_cfg_dict: dict
    :returns: whether or not this "not" argument should be included based
    on the contents of rustc_cfg_dict
    :rtype: bool
    """
    not_arg = not_match.group(1)

    all_match = all_re.match(not_arg)
    if all_match is not None:
        return not process_all(all_match, all_re, any_re, not_re, rustc_cfg_dict)

    any_match = any_re.match(not_arg)
    if any_match is not None:
        return not process_any(any_match, all_re, any_re, not_re, rustc_cfg_dict)

    not_match = not_re.match(not_arg)
    if not_match is not None:
        return not process_not(not_match, all_re, any_re, not_re, rustc_cfg_dict)

    return not process_basic(not_arg, rustc_cfg_dict)


def process_basic(configuration_option, rustc_cfg_dict):
    """
    :param configuration_option: the string representation of the configuration
    option
    :type basic_match: str
    :param rustc_cfg_dict: dict containing information from the output of
    `rustc --print cfg`
    :type rustc_cfg_dict: dict
    :returns: a bool indicating whether or not this "basic" argument should
    be included based on the contents of rustc_cfg_dict
    :rtype: bool
    """
    key_value_pattern = r"([^-]*) = \"([^-]*)\""
    key_value_re = re.compile(key_value_pattern)
    key_value_match = key_value_re.match(configuration_option)

    if key_value_match is not None:
        return bool(
            rustc_cfg_dict[key_value_match.group(1)] == key_value_match.group(2)
        )

    return bool(rustc_cfg_dict["cfg"] == configuration_option)


def build_re_dict(cargo_outdated_platform):
    """
    :param cargo_outdated_platform: the string representation of the
    platform outputted by `cargo outdated`
    :type cargo_outdated_platform: str
    :returns: dict containing regular expression information surrounding
    cargo_outdated_platform
    the keys are the string descriptions of re.Pattern objects and of
    re.Match or NoneType objects
    the values are the re.Pattern objects and the re.Match or NoneType
    objects described by the keys
    :rtype: dict
    """

    re_dict = {}

    all_pattern = r"all\(([^-]*)\)"
    any_pattern = r"any\(([^-]*)\)"
    not_pattern = r"not\(([^-]*)\)"

    re_dict["all_re"] = re.compile(all_pattern)
    re_dict["any_re"] = re.compile(any_pattern)
    re_dict["not_re"] = re.compile(not_pattern)

    re_dict["all_match"] = re_dict["all_re"].match(cargo_outdated_platform)
    re_dict["any_match"] = re_dict["any_re"].match(cargo_outdated_platform)
    re_dict["not_match"] = re_dict["not_re"].match(cargo_outdated_platform)

    return re_dict


def parse_cfg_format_platform(cfg_match, rustc_cfg_dict):
    """
    :param cfg_match: the re.Match representation of the "cfg"-format platform
    to parse
    :type cfg_match: re.Match
    :param rustc_cfg_dict: dict containing information from the output of
    `rustc --print cfg`
    :type rustc_cfg_dict: dict
    :returns: a bool indicating whether or not this "cfg"-format platform should
    be included based on the contents of rustc_cfg_dict
    :rtype: bool
    """
    re_dict = build_re_dict(cfg_match.group(1))

    if re_dict["all_match"] is not None:
        return process_all(
            re_dict["all_match"],
            re_dict["all_re"],
            re_dict["any_re"],
            re_dict["not_re"],
            rustc_cfg_dict,
        )

    if re_dict["any_match"] is not None:
        return process_any(
            re_dict["any_match"],
            re_dict["all_re"],
            re_dict["any_re"],
            re_dict["not_re"],
            rustc_cfg_dict,
        )

    if re_dict["not_match"] is not None:
        return process_not(
            re_dict["not_match"],
            re_dict["all_re"],
            re_dict["any_re"],
            re_dict["not_re"],
            rustc_cfg_dict,
        )

    return process_basic(cfg_match.group(1), rustc_cfg_dict)


def parse_target_format_platform(unparsed_platform, rustc_cfg_dict):
    """
    :param unparsed_platform: the string representation of the "target"-format
    platform to parse
    :type unparsed_platform: str
    :param rustc_cfg_dict: dict containing information from the output of
    `rustc --print cfg`
    :type rustc_cfg_dict: dict
    :returns: a bool indicating whether or not this "target"-format platform
    should be included based on the contents of rustc_cfg_dict
    :rtype: bool
    """
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
    :param unparsed_platform: the string representation of the platform to parse
    :type unparsed_platform: str
    :returns: a bool indicating whether or not this platform should be included
    based on the contents of rustc_cfg_dict
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
    :returns: a dictionary containing information from the output of `cargo
    outdated`
    the keys are the string representations of dependencies
    the values are 4-tuples containing
    1) the string represenation of the dependency's version (i.e. from the
    "Project" column of the output of `cargo outdated`,
    2) the string representation of the dependency the dependency is pulled in by
    or None if the dependency is not pulled in by any dependency
    3) the string representation of the dependency's platform information (i.e.
    from the "Platform" column of the ouput of `cargo outdated`
    4) a bool indicating whether or not the dependency should be "included", with
    respect to the platform information
    :rtype: dict
    """
    cargo_outdated_dict = {}

    command = ["cargo", "outdated"]
    proc = subprocess.Popen(command, stdout=subprocess.PIPE)

    cargo_outdated_pattern = (
        r"([^\s]*)\s*([^\s]*)\s*([^\s]*)\s*([^\s]*)\s*([^\s]*)\s*(.*)"
    )
    cargo_outdated_re = re.compile(cargo_outdated_pattern)

    while True:
        line = proc.stdout.readline()

        if line == b"":
            break

        line_str = line.decode("utf-8")
        cargo_outdated_match = cargo_outdated_re.match(line_str)

        if cargo_outdated_match.group(1) in ("Name", "----"):
            continue

        dependencies = cargo_outdated_match.group(1)
        dependencies_split = dependencies.split("->")

        dependency = dependencies_split.pop(-1)

        version = cargo_outdated_match.group(2)
        pulled_in_by = None if dependencies_split == [] else dependencies_split[0]
        platform = cargo_outdated_match.group(6)
        include = parse_platform(platform)

        cargo_outdated_dict[dependency] = (
            version,
            pulled_in_by,
            platform,
            include,
        )

    return cargo_outdated_dict


def build_koji_repo_dict(cargo_outdated_dict):
    """
    :param cargo_outdated_dict: a dictionary containing information from the
    output of `cargo outdated`
    the keys are the string representations of dependencies
    the values are 4-tuples containing
    1) the string represenation of the dependency's version (i.e. from the
    "Project" column of the output of `cargo outdated`,
    2) the string representation of the dependency the dependency is pulled in by
    or None if the dependency is not pulled in by any dependency
    3) the string representation of the dependency's platform information (i.e.
    from the "Platform" column of the ouput of `cargo outdated`
    4) a bool indicating whether or not the dependency should be "included", with
    respect to the platform information
    :type cargo_outdated_dict: dict
    :returns: a dictioonary containing information from the koji repo webpage
    the keys are the string representations of dependencies
    the values are the string representations of versions of dependencies
    :rtype: dict
    """
    koji_repo_dict = {}

    requests_var = requests.get(
        "https://kojipkgs.fedoraproject.org/repos/rawhide/latest/x86_64/pkglist"
    )
    packages = requests_var.text

    pattern = r"^toplink/packages/(rust-)?([^\/]*?)/([^\/]*?)/[^]*)]*"
    my_reg_ex = re.compile(pattern)

    for line in packages.splitlines():
        matches = my_reg_ex.match(line)
        if matches.group(2) in cargo_outdated_dict.keys():
            koji_repo_dict[matches.group(2)] = matches.group(3)

    return koji_repo_dict


def build_and_print_command(outdated_dict):
    """
    :param outdated_dict: the dictionary of the string representations
    of outdated dependencies and the string representations of the
    versions they ought to be updated to
    :type outdated_dict: dict
    """

    command = ""
    for key in outdated_dict:
        command += "cargo update -p {} --precise {}\n".format(key, outdated_dict[key])

    print("\n\nUSE THESE COMMANDS TO UPDATE PACKAGES\n")

    print(command)


def print_verbose_results(table_data):
    """
    :param table_data: the data to be printed out in a tablular format
    :type table_data: list of lists
    """
    print("\n\nVERBOSE RESULTS\n")

    for row in table_data:
        print("{: <30} {: <10} {: <10} {: <10} {: <10} {: <30}".format(*row))


def print_results(results):
    """
    :param results: a 4-tuple containing:
    1) the dictionary of the string representations of outdated dependencies and the
    string representations of the versions they ought to be updated to
    2) the list of the string representations of the not-outdated dependencies
    3) the list of the string representations of the not-found dependencies
    4) the list of the string representations of the not-included dependencies
    :type results: 5-tuple of dict, list, list, list, list
    """
    print("\n\nRESULTS")

    print(
        "\nThe following crates that were outputted by 'cargo outdated' are outdated"
        " with respect to the koji repo, and should be updated to the following versions:"
    )
    print(results[0])

    print(
        "\nThe following crates that were outputted by 'cargo outdated' are not outdated"
        " with respect to the koji repo:"
    )
    print(results[1])

    print(
        "\nThe following crates that were outputted by 'cargo outdated' were not found"
        " in the koji repo:"
    )
    print(results[2])

    print(
        "\nThe following crates that were outputted by 'cargo outdated' have an irrelevant"
        " platform and may or may not be outdated:"
    )
    print(results[3])


def get_overall_include(cargo_outdated_dict, key):
    """
    :param cargo_outdated_dict: a dictionary containing information from the
    output of `cargo outdated`
    the keys are the string representations of dependencies
    the values are 4-tuples containing
    1) the string represenation of the dependency's version (i.e. from the
    "Project" column of the output of `cargo outdated`,
    2) the string representation of the dependency the dependency is pulled in by
    or None if the dependency is not pulled in by any dependency
    3) the string representation of the dependency's platform information (i.e.
    from the "Platform" column of the ouput of `cargo outdated`
    4) a bool indicating whether or not the dependency should be "included", with
    respect to the platform information
    :type cargo_outdated_dict: dict
    :param koji_repo_dict: a dictionary containing information from the koji repo webpage
    the keys are the string representations of dependencies
    the values are the string representations of versions of dependencies
    :type koji_repo_dict: dict
    :returns: a bool indicating whether or not the key should be included with all the
    dependencies it depends on taken into account
    :rtype: bool
    """

    current_key = key
    pulled = cargo_outdated_dict[current_key][1]
    include = cargo_outdated_dict[current_key][3]

    if not include:
        return False

    while pulled in cargo_outdated_dict and cargo_outdated_dict[pulled] is not None:
        if not cargo_outdated_dict[pulled][3]:
            return False
        current_key = pulled
        pulled = cargo_outdated_dict[current_key][1]

    return True


def build_results(cargo_outdated_dict, koji_repo_dict):
    """
    :param cargo_outdated_dict: a dictionary containing information from the
    output of `cargo outdated`
    the keys are the string representations of dependencies
    the values are 4-tuples containing
    1) the string represenation of the dependency's version (i.e. from the
    "Project" column of the output of `cargo outdated`,
    2) the string representation of the dependency the dependency is pulled in by
    or None if the dependency is not pulled in by any dependency
    3) the string representation of the dependency's platform information (i.e.
    from the "Platform" column of the ouput of `cargo outdated`
    4) a bool indicating whether or not the dependency should be "included", with
    respect to the platform information
    :type cargo_outdated_dict: dict
    :param koji_repo_dict: a dictionary containing information from the koji repo webpage
    the keys are the string representations of dependencies
    the values are the string representations of versions of dependencies
    :type koji_repo_dict: dict
    :returns: the results in the form of a tuple
    :rtype: 2-tuple of tuple, list
    """

    outdated = {}
    not_outdated = []
    not_found = []
    not_included = []
    table_data = []

    table_data.append(
        ["Crate", "Outdated", "Current", "Update To", "Include", "Platform",]
    )
    table_data.append(
        ["-----", "--------", "-------", "---------", "-------", "--------",]
    )

    for key in cargo_outdated_dict:

        version = cargo_outdated_dict[key][0]
        pulled_in_by = cargo_outdated_dict[key][1]
        platform = cargo_outdated_dict[key][2]

        # Key was not found.
        if key not in koji_repo_dict.keys():
            not_found.append(key)

            table_data.append([key, "-", version, "-", "N/A", platform])
            continue

        overall_include = get_overall_include(cargo_outdated_dict, key)

        if overall_include:
            overall_include_str = "Yes"
        else:
            overall_include_str = "No"

        # Key is outdated.
        if koji_repo_dict[key] != version:

            if pulled_in_by in cargo_outdated_dict:
                if cargo_outdated_dict[pulled_in_by][3] and overall_include:
                    outdated[key] = koji_repo_dict[key]
                    table_data.append(
                        [
                            key,
                            "Yes",
                            version,
                            koji_repo_dict[key],
                            overall_include_str,
                            platform,
                        ]
                    )

                if cargo_outdated_dict[pulled_in_by][3] and not overall_include:
                    not_included.append(key)
                    table_data.append(
                        [
                            key,
                            "Yes",
                            version,
                            koji_repo_dict[key],
                            overall_include_str,
                            platform,
                        ]
                    )

        # Key is up-to-date.
        if koji_repo_dict[key] == version:
            if pulled_in_by in cargo_outdated_dict:

                if cargo_outdated_dict[pulled_in_by][3] and overall_include:
                    not_outdated.append(key)
                    table_data.append(
                        [
                            key,
                            "No",
                            version,
                            koji_repo_dict[key],
                            overall_include_str,
                            platform,
                        ]
                    )

                if cargo_outdated_dict[pulled_in_by][3] and not overall_include:
                    not_included.append(key)
                    table_data.append(
                        [
                            key,
                            "No",
                            version,
                            koji_repo_dict[key],
                            overall_include_str,
                            platform,
                        ]
                    )

    return (
        (outdated, not_outdated, not_found, not_included),
        table_data,
    )


def main():
    """
    The main method
    """
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "-v",
        "--verbose",
        help="print table with more detailed information",
        dest="verbose",
        action="store_true",
    )
    parser.add_argument(
        "-c",
        "--command",
        help="print command(s) that would update crates as necessary",
        dest="command",
        action="store_true",
    )
    args = parser.parse_args()

    cargo_outdated_dict = build_cargo_outdated_dict()
    koji_repo_dict = build_koji_repo_dict(cargo_outdated_dict)
    results = build_results(cargo_outdated_dict, koji_repo_dict)
    print_results(results[0])

    if args.command:
        build_and_print_command(results[0][0])

    if args.verbose:
        print_verbose_results(results[1])


if __name__ == "__main__":
    sys.exit(main())
