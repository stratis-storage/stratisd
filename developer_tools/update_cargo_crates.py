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
import re
import subprocess
import sys
from collections import defaultdict, namedtuple

# isort: THIRDPARTY
import requests
from networkx import MultiDiGraph
from networkx.algorithms.dag import all_topological_sorts
from networkx.drawing.nx_pydot import write_dot
from semantic_version import Spec, Version

PLATFORM = "unix"
OS = "linux"

CARGO_OUTDATED_PLACE_HOLDER = "---"
CARGO_OUTDATED_REMOVED = "Removed"

CARGO_OUTDATED_RE = re.compile(
    # pylint: disable=line-too-long
    r"(?P<name>[^\s]*)\s*(?P<project>[^\s]*)\s*(?P<compat>[^\s]*)\s*(?P<latest>[^\s]*)\s*(?P<kind>[^\s]*)\s*(?P<platform>.*)"
)
CARGO_TREE_RE = re.compile(r"(?P<crate>[a-z0-9_\-]+) v(?P<version>[0-9\.]+)( \(.*\))?$")
CFG_RE = re.compile(r"cfg\((?P<body>([^-]*))\)$")
KEY_VALUE_RE = re.compile(r"(?P<key>.*) = \"(?P<value>.*)\"$")
KOJI_RE = re.compile(
    r"^toplink/packages/rust-(?P<name>[^\/]*?)/(?P<version>[^\/]*?)/[^]*)]*"
)
RE_DICT = {
    "all_re": re.compile(r"all\(([^-]*)\)$"),
    "any_re": re.compile(r"any\(([^-]*)\)$"),
    "not_re": re.compile(r"not\(([^-]*)\)$"),
}
VERSION_FMT_STR = "{} ({})"

Results = namedtuple(
    "Results",
    [
        "uninteresting",
        "up_to_date",
        "not_found",
        "auto_update",
        "fedora_up_to_date",
        "requires_edit",
    ],
)


def _check_cfg(configuration_option):
    """
    :param configuration_option: the string representation of the configuration
    option
    :type configuration option: str
    :returns: a bool indicating whether or not this configuration option
    indicates that the dependency is relevant. May return a false positive if
    the configuration option is sufficiently complex.
    :rtype: bool
    """
    if any(re.match(configuration_option) is not None for re in RE_DICT.values()):
        return True

    key_value_match = KEY_VALUE_RE.match(configuration_option)
    if key_value_match is not None:
        return (
            key_value_match.group("value") == OS
            if key_value_match.group("key") == "target_os"
            else True
        )

    return configuration_option == PLATFORM


def _check_relevance(platform):
    """
    Determines whether the platform is relevant. Returns True in case of
    uncertainty.

    :param str platform: the string representation of the platform annotation
    :returns: True if the annotation indicates that the crate is relevant
    :rtype: bool
    """

    if platform == CARGO_OUTDATED_PLACE_HOLDER:
        return True

    cfg_match = CFG_RE.match(platform)
    if cfg_match is not None:
        return _check_cfg(cfg_match.group("body"))

    target_components = platform.split("-")
    return len(target_components) == 4 and target_components[2] == OS


def _build_cargo_tree_dict():
    """
    Build a map of crate names to versions from the output of cargo tree

    :returns: a map from crates names to sets of versions
    :rtype: dict of str * set of Version
    """
    command = ["cargo", "tree"]
    proc = subprocess.Popen(command, stdout=subprocess.PIPE)

    stream = proc.stdout
    stream.readline()  # omit libstratis, it's not packaged for Fedora

    version_dict = defaultdict(set)
    while True:
        line = stream.readline()

        if line == b"":
            break

        line_str = line.decode("utf-8").strip()
        if (
            # pylint: disable=bad-continuation
            line_str.find("build-dependencies") != -1
            or line_str.find("dev-dependencies") != -1
        ):
            continue
        cargo_tree_match = CARGO_TREE_RE.search(line_str)
        assert cargo_tree_match is not None, line_str
        version_dict[cargo_tree_match.group("crate")].add(
            Version(cargo_tree_match.group("version"), partial=True)
        )

    return version_dict


def _build_cargo_outdated_graph():
    """
    :returns: A graph representation of `cargo updated` output
    :rtype: MultiDiGraph
    """
    command = ["cargo", "outdated"]
    proc = subprocess.Popen(command, stdout=subprocess.PIPE)

    stream = proc.stdout

    stream.readline()
    stream.readline()

    dependency_graph = MultiDiGraph()
    while True:
        line = stream.readline()

        if line == b"":
            break

        line_str = line.decode("utf-8")
        cargo_outdated_match = CARGO_OUTDATED_RE.match(line_str)

        dependencies = cargo_outdated_match.group("name")
        dependencies_split = dependencies.split("->")
        dependency = dependencies_split.pop(-1)
        pulled_in_by = None if dependencies_split == [] else dependencies_split[0]
        if pulled_in_by is None:
            dependency_graph.add_node(
                dependency,
                platform=cargo_outdated_match.group("platform"),
                project=cargo_outdated_match.group("project"),
                compat=cargo_outdated_match.group("compat"),
                latest=cargo_outdated_match.group("latest"),
                kind=cargo_outdated_match.group("kind"),
            )
        else:
            dependency_graph.add_edge(
                pulled_in_by,
                dependency,
                platform=cargo_outdated_match.group("platform"),
                project=cargo_outdated_match.group("project"),
                compat=cargo_outdated_match.group("compat"),
                latest=cargo_outdated_match.group("latest"),
                kind=cargo_outdated_match.group("kind"),
            )

    return dependency_graph


def _build_koji_repo_dict(cargo_tree):
    """
    :param cargo_tree: a table of crates and versions
    :type cargo_tree: dict of str * set of Version
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
        if name in cargo_tree:
            koji_repo_dict[name] = Version(matches.group("version"), partial=True)

    # Post-condition: koji_repo_dict.keys() <= cargo_tree.keys().
    # cargo tree may show internal dependencies that are not separate packages
    return koji_repo_dict


def _make_spec(project, compat):
    """
    Make a version specification for the range between project and compatible
    versions. Handle special symbols.

    :param str project: the curent project version
    :param str compat: the highest compatible project version
    :returns: the spec
    :rtype: Spec
    """
    return Spec(
        "==%s" % project
        if compat == CARGO_OUTDATED_PLACE_HOLDER
        else ">=%s,<=%s" % (project, compat)
    )


def _build_edge(edge):
    """
    Build a multigraph graph edge.

    Since there may be multiple edges between the same nodes, there is a
    third entry, which is the index. Mostly there is only one edge, and in
    that case the index is left out and has to be appended.

    :returns: a triple represeting an edge
    :rtype: tuple of str * str * int
    """
    edge = list(edge)
    if len(edge) == 2:
        edge.append(0)
    return edge


def _removable_node(graph, node):
    """
    Determine if a node can be removed from the graph.

    It can be if it is descended only from nodes that can be removed.

    If it is not descended from any nodes it can be removed if it is itself
    removable.

    Precondition: the node is a leaf
    :return: True if the node is irrelevant to Fedora packaging, else False
    :rtype: bool
    """
    if list(graph.in_edges(node)) == []:
        platform = graph.nodes[node].get("platform")
        return False if platform is None else not _check_relevance(platform)

    result = True
    for edge in graph.in_edges(node):
        edge = _build_edge(edge)
        attrs = graph.edges[edge[0], edge[1], edge[2]]
        if _check_relevance(attrs["platform"]) and not _removable_node(graph, edge[0]):
            return False

    return result


def _strip_irrelevant_nodes(cargo_outdated):
    """
    Remove nodes that are irrelevant to Fedora.

    :returns: a list of the nodes removed
    :rtype: list of str
    """

    uninteresting = []
    while True:
        remove = None
        for node in cargo_outdated.nodes:
            if list(cargo_outdated.successors(node)) != []:
                continue

            if _removable_node(cargo_outdated, node):
                remove = node
                break
            remove = None

        if remove is None:
            break

        uninteresting.append(remove)
        cargo_outdated.remove_node(remove)

    return uninteresting


def _find_up_to_date_nodes(cargo_outdated):
    """
    Find crates that are entirely up-to-date with the version published on
    crates.io.

    :returns: a list of the up-to-date crates
    :rtype: set of str
    """
    up_to_date = set()
    for node in cargo_outdated.nodes:
        if list(cargo_outdated.predecessors(node)) != []:
            continue
        # found root of a graph, has no outdated parents, has no entry for
        # itself.
        if cargo_outdated.nodes[node] == {}:
            up_to_date.add(node)

    return up_to_date


def _find_root_nodes(cargo_outdated):
    """
    Find all root nodes listed in the cargo outdated output.
    These represent crates specified in the Cargo.toml file for this project.

    :returns: a set of the outdated root nodes
    :rtype: set of str
    """
    root = set()
    for node in cargo_outdated.nodes:
        if list(cargo_outdated.predecessors(node)) != []:
            continue
        if cargo_outdated.nodes[node] != {}:
            root.add(node)

    return root


def _get_current_versions(cargo_outdated, node):
    """
    Get all current versions for a crate

    Precondition: node is not up-to-date, so it either has parents or it
    has attributes.

    :returns: the set of current versions
    :rtype: set of str
    """
    current_versions = set()
    for edge in cargo_outdated.in_edges(node):
        edge = _build_edge(edge)
        attributes = cargo_outdated.edges[edge[0], edge[1], edge[2]]
        current_versions.add(attributes["project"])

    return (
        set([cargo_outdated.nodes[node]["project"]])
        if current_versions == set()
        else current_versions
    )


def _get_specs(cargo_outdated, node):
    """
    Get the spec ranges from current to highest available auto-updatable

    Precondition: node is not up-to-date, so it either has parents or it
    has attributes.

    :returns: the set of current specs. If the second part is "Removed"
    omits the spec.

    :returns: a set of specs
    :rtype: set of Spec
    """
    specs = set()
    for edge in cargo_outdated.in_edges(node):
        edge = _build_edge(edge)
        attributes = cargo_outdated.edges[edge[0], edge[1], edge[2]]
        compat = attributes["compat"]
        if compat != CARGO_OUTDATED_REMOVED:
            specs.add(_make_spec(attributes["project"], compat))

    if specs == set():
        attributes = cargo_outdated.nodes[node]
        # if this node has parents, but the only relevant ones have a
        # Removed relationship in the second part of the spec, then
        # it's possible for specs to be still empty even though the node
        # is not up-to-date. So, it is necessary to check for empty
        # attributes.
        if attributes == {}:
            return set()
        compat = attributes["compat"]
        if compat != CARGO_OUTDATED_REMOVED:
            return set([_make_spec(attributes["project"], compat)])
        return set()

    return specs


def _build_results(cargo_outdated, koji_repo_dict):
    """
    Precondition: It has already been verified that cargo_outdated contains
    no crates with versions greater than the Fedora version.

    :param cargo_outdated: information from the output of `cargo outdated`
    :type cargo_outdated: MultiDiGraph
    :param koji_repo_dict: a dictionary containing information from the koji repo webpage
    the keys are the string representations of dependencies
    the values are the string representations of versions of dependencies
    :type koji_repo_dict: dict of str * Version
    :returns: the results in the form of a tuple
    :rtype: 2-tuple of tuple, list
    """

    uninteresting = _strip_irrelevant_nodes(cargo_outdated)

    up_to_date = _find_up_to_date_nodes(cargo_outdated)

    not_found = set(
        node
        for node in cargo_outdated.node
        if node not in koji_repo_dict and node not in up_to_date
    )

    auto_update = {}
    fedora_up_to_date = {}
    requires_edit = {}
    for node in cargo_outdated.nodes:
        if node in up_to_date or node in not_found:
            continue

        current_versions = _get_current_versions(cargo_outdated, node)

        koji_version = koji_repo_dict[node]
        if all(koji_version == Version(version) for version in current_versions):
            fedora_up_to_date[node] = koji_version
            continue

        assert all(koji_version >= Version(version) for version in current_versions)

        specs = _get_specs(cargo_outdated, node)

        if all(koji_version in spec for spec in specs):
            auto_update[node] = koji_version
            continue

        requires_edit[node] = koji_version

    return Results(
        uninteresting=uninteresting,
        up_to_date=up_to_date,
        not_found=not_found,
        auto_update=auto_update,
        fedora_up_to_date=fedora_up_to_date,
        requires_edit=requires_edit,
    )


def main():  # pylint: disable=too-many-branches, too-many-locals, too-many-statements
    """
    The main method
    """
    parser = argparse.ArgumentParser(
        description=(
            "A script to process the output of the cargo-outdated command and "
            "information about Fedora packages in order to make "
            "recommendations about desirable updates"
        )
    )
    parser.add_argument(
        "--auto-update",
        help=(
            "Print crates which can be automatically updated to the current "
            "Fedora version using cargo-update. If list is specified print "
            "a list, if command, print cargo-update commands instead"
        ),
        dest="auto_update",
        action="store",
        choices=["list", "command"],
    )
    parser.add_argument(
        "--dependent",
        help=(
            "Print crates reported by the cargo-outdated command that "
            "appear to be pinned to a low version by a dependent crate of this "
            "project"
        ),
        dest="dependent",
        action="store_true",
    )
    parser.add_argument(
        "--fedora-up-to-date",
        help=(
            "Print crates reported by the cargo-outdated command that "
            "have the same version as the Fedora rawhide package but are not "
            "up-to-date with respect to crates.io"
        ),
        dest="futd",
        action="store_true",
    )
    parser.add_argument(
        "--irrelevant",
        help=(
            "Print crates reported by the cargo-outdated command but "
            "actually irrelevant, since they are included only on non-Fedora "
            "platforms, e.g., windows"
        ),
        dest="irrelevant",
        action="store_true",
    )
    parser.add_argument(
        "--manual",
        help=(
            "Print crates reported by the cargo-outdated command that lag "
            "the versions in Fedora but cannot be updated via cargo-update. "
            "Print only those that can be changed by editing the Cargo.toml "
            "for this project."
        ),
        dest="manual",
        action="store_true",
    )
    parser.add_argument(
        "--not-found",
        help=(
            "Print crates reported by the cargo-outdated command but "
            "apparently not available on Fedora"
        ),
        dest="missing",
        action="store_true",
    )
    parser.add_argument(
        "--search",
        help=("Find out how packages are classified at this time"),
        dest="search",
        default=[],
        action="append",
    )
    parser.add_argument(
        "--up-to-date",
        help=(
            "Print crates reported by the cargo-outdated command that are "
            "entirely up-to-date with respect to the version released on "
            "crates.io"
        ),
        dest="up_to_date",
        action="store_true",
    )
    parser.add_argument(
        "--write",
        help=("Export dependency graph to dot format"),
        dest="write",
        action="store_true",
    )
    args = parser.parse_args()

    cargo_tree = _build_cargo_tree_dict()
    koji_repo_dict = _build_koji_repo_dict(cargo_tree)

    for crate, versions in cargo_tree.items():
        koji_version = koji_repo_dict.get(crate)
        if koji_version is None:
            continue
        if any(version > koji_version for version in versions):
            print(
                "Some version of crate %s higher than maximum %s"
                % (crate, koji_version),
                file=sys.stderr,
            )
            return 1

    cargo_outdated = _build_cargo_outdated_graph()
    result = _build_results(cargo_outdated, koji_repo_dict)

    node_count = len(list(cargo_outdated.nodes))
    result_count = sum(
        len(getattr(result, res)) for res in result._fields if res != "uninteresting"
    )

    if node_count != result_count:
        print(
            "node count %s != result count %s" % (node_count, result_count),
            file=sys.stderr,
        )
        return 1

    if args.auto_update:
        if args.auto_update == "command":
            fmt_str = "cargo update -p {} --precise {}"
        else:
            fmt_str = VERSION_FMT_STR

        subgraph = cargo_outdated.subgraph(result.auto_update.keys())
        for crate in reversed(next(all_topological_sorts(subgraph))):
            print(fmt_str.format(crate, result.auto_update[crate]))

    if args.dependent:
        root_nodes = _find_root_nodes(cargo_outdated)
        for crate, version in sorted(result.requires_edit.items()):
            if crate not in root_nodes:
                print(VERSION_FMT_STR.format(crate, version))

    if args.futd:
        for crate, version in sorted(result.fedora_up_to_date.items()):
            print(VERSION_FMT_STR.format(crate, version))

    if args.irrelevant:
        for crate in sorted(result.uninteresting):
            print(crate)

    if args.manual:
        root_nodes = _find_root_nodes(cargo_outdated)
        for crate, version in sorted(result.requires_edit.items()):
            if crate in root_nodes:
                print(VERSION_FMT_STR.format(crate, version))

    if args.missing:
        for crate in sorted(result.not_found):
            print(crate)

    if args.search != []:
        for crate in sorted(args.search):
            owners = [
                field for field in result._fields if crate in getattr(result, field)
            ]
            assert len(owners) in (0, 1), owners

            if owners == []:
                print("{} not found".format(crate))
            else:
                print("{} in {}".format(crate, owners[0]))

    if args.up_to_date:
        for crate in sorted(result.up_to_date):
            print(crate)

    if args.write:
        write_dot(cargo_outdated, sys.stdout)

    return 0


if __name__ == "__main__":
    sys.exit(main())
