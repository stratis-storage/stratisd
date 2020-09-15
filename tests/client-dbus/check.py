#!/usr/bin/python

# isort: STDLIB
import argparse
import subprocess
import sys

arg_map = {
    "src/stratisd_client_dbus": [
        "--reports=no",
        "--disable=I",
        "--disable=invalid-name",
        "--ignore=_data.py",  # ignore _data.py because it is auto-generated
        "--msg-template='{path}:{line}: [{msg_id}({symbol}), {obj}] {msg}'",
    ],
    "tests": [
        "--reports=no",
        "--disable=I",
        "--disable=duplicate-code",
        "--msg-template='{path}:{line}: [{msg_id}({symbol}), {obj}] {msg}'",
    ],
}


def get_parser():
    """
    Generate an appropriate parser.

    :returns: an argument parser
    :rtype: `ArgumentParser`
    """
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "package", choices=arg_map.keys(), help="designates the package to test"
    )
    parser.add_argument("--ignore", help="ignore these files")
    return parser


def get_command(namespace):
    """
    Get the pylint command for these arguments.

    :param `Namespace` namespace: the namespace
    """
    cmd = ["pylint", namespace.package] + arg_map[namespace.package]
    if namespace.ignore:
        cmd.append("--ignore=%s" % namespace.ignore)
    return cmd


def main():
    args = get_parser().parse_args()
    return subprocess.call(get_command(args), stdout=sys.stdout)


if __name__ == "__main__":
    sys.exit(main())
