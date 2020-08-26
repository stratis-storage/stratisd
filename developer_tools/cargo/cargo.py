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

    pattern = r'target_([^\s]*)="([^\s]*)"'
    my_reg_ex = re.compile(pattern)

    while True:
        line = proc.stdout.readline()

        if line == b"":
            break

        line_str = line.decode("utf-8")
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
    :param all_match: a match to the compiled "all" regular expression
    :type all_match: re.Match
    :param not_re: the compiled "not" regular expression
    :type not_re: re.Pattern
    :param basic_re: the compiled "basic" regular expression
    :type basic_re: re.Pattern
    :param rustc_cfg_dict: dict containing information from the output of
    `rustc --print cfg`
    :type rustc_cfg_dict: dict
    :returns: a bool indicating whether or not this "all" argument should
    be included based on the contents of rustc_cfg_dict
    :rtype: bool
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
    :param any_match: a match to the compiled "any" regular expression
    :type any_match: re.Match
    :param not_re: the compiled "not" regular expression
    :type not_re: re.Pattern
    :param basic_re: the compiled "basic" regular expression
    :type basic_re: re. Pattern
    :param rustc_cfg_dict: dict containing information from the output of
    `rustc --print cfg`
    :type rustc_cfg_dict: dict
    :returns: a bool indicating whether or not this "any" argument should
    be included based on the contents of rustc_cfg_dict
    :rtype: bool
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
    :param basic_match: a match to the compiled "basic" regular expression
    :type basic_match: re.Match
    :param rustc_cfg_dict: dict containing information from the output of
    `rustc --print cfg`
    :type rustc_cfg_dict: dict
    :returns: a bool indicating whether or not this "basic" argument should
    be included based on the contents of rustc_cfg_dict
    :rtype: bool
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
    :param not_match: a match to the compiled "not" regular expression
    :type not_match: re.Match
    :param rustc_cfg_dict: dict containing information from the output of
    `rustc --print cfg`
    :type rustc_cfg_dict: dict
    :returns: whether or not this "not" argument should be included based
    on the contents of rustc_cfg_dict
    :rtype: bool
    """
    return not process_basic(not_match, rustc_cfg_dict)


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
    basic_pattern = r"([^-]*)"

    re_dict["all_re"] = re.compile(all_pattern)
    re_dict["any_re"] = re.compile(any_pattern)
    re_dict["not_re"] = re.compile(not_pattern)
    re_dict["basic_re"] = re.compile(basic_pattern)

    re_dict["all_match"] = re_dict["all_re"].match(cargo_outdated_platform)
    re_dict["any_match"] = re_dict["any_re"].match(cargo_outdated_platform)
    re_dict["not_match"] = re_dict["not_re"].match(cargo_outdated_platform)
    re_dict["basic_match"] = re_dict["basic_re"].match(cargo_outdated_platform)

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

    pattern = r"([^\s]*)\s*([^\s]*)\s*([^\s]*)\s*([^\s]*)\s*([^\s]*)\s*(.*)"
    my_reg_ex = re.compile(pattern)

    while True:
        line = proc.stdout.readline()

        if line == b"":
            break

        line_str = line.decode("utf-8")
        matches = my_reg_ex.match(line_str)

        if matches.group(1) in ("Name", "----"):
            continue

        platform = matches.group(6)
        include = parse_platform(platform)

        dependencies = matches.group(1)
        version = matches.group(2)

        if "->" not in dependencies:
            dependency = dependencies
            cargo_outdated_dict[dependency] = (version, None, platform, include)
        else:
            dependencies_split = dependencies.split("->")
            pulled_in_by = dependencies_split[0]
            dependency = dependencies_split[1]
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


def print_results(cargo_outdated_dict, koji_repo_dict):
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
    """

    outdated = []
    not_outdated = []
    not_found = []
    not_included = []
    table_data = []

    table_data.append(
        ["Crate", "Outdated?", "Current", "Update To", "Include?", "Platform"]
    )
    table_data.append(
        ["-----", "---------", "-------", "---------", "--------", "--------"]
    )

    for key in cargo_outdated_dict:

        version = cargo_outdated_dict[key][0]
        platform = cargo_outdated_dict[key][2]
        include = cargo_outdated_dict[key][3]

        if key in koji_repo_dict.keys():
            if koji_repo_dict[key] != version:
                table_data.append(
                    [
                        key,
                        "Outdated",
                        version,
                        koji_repo_dict[key],
                        str(include),
                        platform,
                    ]
                )
                if include:
                    outdated.append(key)
                else:
                    not_included.append(key)

            else:
                table_data.append(
                    [
                        key,
                        "Not Outdated",
                        version,
                        koji_repo_dict[key],
                        str(include),
                        platform,
                    ]
                )
                if include:
                    not_outdated.append(key)
                else:
                    not_included.append(key)

        else:
            table_data.append(
                [key, "Not Found", version, "---", str(include), platform]
            )
            if include:
                not_found.append(key)
            else:
                not_included.append(key)

    print("\n\nRESULTS")

    print(
        "\nThe following crates that were outputted by 'cargo outdated' are outdated"
        " with respect to the koji repo:"
    )
    print(outdated)

    print(
        "\nThe following crates that were outputted by 'cargo outdated' are not outdated"
        " with respect to the koji repo:"
    )
    print(not_outdated)

    print(
        "\nThe following crates that were outputted by 'cargo outdated' were not found"
        " in the koji repo:"
    )
    print(not_found)

    print(
        "\nThe following crates that were outputted by 'cargo outdated' have an irrelevant"
        " platform and may or may not be outdated:"
    )
    print(not_included)

    print("\n\nVERBOSE RESULTS\n")

    for row in table_data:
        print("{: <30} {: <15} {: <10} {: <10} {: <10} {: <30}".format(*row))


def main():
    """
    The main method
    """
    cargo_outdated_dict = build_cargo_outdated_dict()
    koji_repo_dict = build_koji_repo_dict(cargo_outdated_dict)
    print_results(cargo_outdated_dict, koji_repo_dict)


if __name__ == "__main__":
    sys.exit(main())
