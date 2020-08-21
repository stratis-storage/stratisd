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
Parse platform column of 'cargo outdated' output
"""
# isort: STDLIB
import re
import subprocess

#!/usr/bin/python


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


# import cargo outdated script as a module, call method as a module in the parsing. interact with it


def process_all(
    all_pattern_match, not_pattern_reg_ex, basic_pattern_reg_ex, rustc_cfg_dict
):
    """
    todo
    """
    all_args = []
    for i in range(1, all_pattern_match.groups):
        all_args.append(all_pattern_match.group(i))

    for all_arg in all_args:
        not_pattern_match = not_pattern_reg_ex.match(all_arg)
        if not_pattern_match is not None:
            if (
                process_not(not_pattern_match, basic_pattern_match, rustc_cfg_dict)
                is True
            ):
                continue
        else:
            basic_pattern_match = basic_pattern_reg_ex.match(all_arg)
            if process_basic(basic_pattern_match, rustc_cfg_dict) is False:
                return False
    return True


def process_any(
    any_pattern_match, not_pattern_reg_ex, basic_pattern_reg_ex, rustc_cfg_dict
):
    """
    todo
    """
    any_args = []
    for i in range(1, any_pattern_match.groups):
        any_args.append(any_pattern_match.group(i))

    for any_arg in any_args:
        not_pattern_match = not_pattern_reg_ex.match(any_arg)
        if not_pattern_match is not None:
            if (
                process_not(not_pattern_match, basic_pattern_match, rustc_cfg_dict)
                is True
            ):
                return True
        else:
            basic_pattern_match = basic_pattern_reg_ex.match(any_arg)
            if process_basic(basic_pattern_match, rustc_cfg_dict) is True:
                return True
    return False


def process_basic(basic_pattern_match, rustc_cfg_dict):
    """
    todo
    """
    return bool(rustc_cfg_dict[basic_pattern_match(1)] is basic_pattern_match(2))


def process_not(not_pattern_match, basic_pattern_match, rustc_cfg_dict):
    """
    todo
    """
    return bool(rustc_cfg_dict[not_pattern_match(1)] is basic_pattern_match(2))


def build_reg_ex_info(to_parse):
    """
    todo
    """
    all_pattern = r"all(([^)]+), ([^)]+)))"
    any_pattern = r"any(([^)]+), ([^)]+)))"
    not_pattern = r"not([^)]+)"
    basic_pattern = r'([^)]+) = "([^)]+)"'

    all_pattern_reg_ex = re.compile(all_pattern)
    any_pattern_reg_ex = re.compile(any_pattern)
    not_pattern_reg_ex = re.compile(not_pattern)
    basic_pattern_reg_ex = re.compile(basic_pattern)

    all_pattern_match = all_pattern_reg_ex.match(to_parse)
    any_pattern_match = any_pattern_reg_ex.match(to_parse)
    not_pattern_match = not_pattern_reg_ex.match(to_parse)
    basic_pattern_match = basic_pattern_reg_ex.match(to_parse)

    reg_exes = [
        all_pattern_reg_ex,
        any_pattern_reg_ex,
        not_pattern_reg_ex,
        basic_pattern_reg_ex,
    ]
    matches = [
        all_pattern_match,
        any_pattern_match,
        not_pattern_match,
        basic_pattern_match,
    ]

    return [reg_exes, matches]


def parse_platform(unparsed_platform):
    """
    :param unparsed_platform: platform to parse
    :type unparsed_platform:
    :returns: duple containing extracted information and whether or not it's a triple
    :rtype: bool
    """

    cfg_pattern = r"cfg\(([^\)]+)\)"
    cfg_reg_ex = re.compile(cfg_pattern)
    cfg_match = cfg_reg_ex.match(unparsed_platform)

    if cfg_match is not None:
        to_parse = cfg_match.group(1)

    else:
        target = unparsed_platform

        target_pattern = r"([^)]+)-([^)]+)-([^)]+)-([^)]+)"
        target_pattern_reg_ex = re.compile(target_pattern)
        target_pattern_match = target_pattern_reg_ex.match(target)

        to_parse = (
            "target_arch = "
            + target_pattern_match.group(1)
            + ", target_vendor = "
            + target_pattern_match.group(2)
            + ", target_os = "
            + target_pattern_match.group(3)
            + ", + target_env = "
            + target_pattern_match.group(4)
        )

    reg_ex_info = build_reg_ex_info(to_parse)
    rustc_cfg_dict = build_rustc_cfg_dict()

    if reg_ex_info[1][0] is not None:
        return process_all(
            reg_ex_info[1][0], reg_ex_info[0][2], reg_ex_info[0][3], rustc_cfg_dict
        )

    if reg_ex_info[1][1] is not None:
        return process_any(
            reg_ex_info[1][1], reg_ex_info[0][2], reg_ex_info[0][3], rustc_cfg_dict
        )

    if reg_ex_info[1][2] is not None:
        return process_not(reg_ex_info[1][2], reg_ex_info[1][3], rustc_cfg_dict)

    if reg_ex_info[1][3] is not None:
        return process_basic(reg_ex_info[1][3], rustc_cfg_dict)

    return True
