# isort: STDLIB
import os
import sys

# isort: THIRDPARTY
import setuptools

if sys.version_info[0] < 3:
    # isort: STDLIB
    from codecs import open


def local_file(name):
    return os.path.relpath(os.path.join(os.path.dirname(__file__), name))


README = local_file("README.rst")

setuptools.setup(
    name="stratisd-client-dbus",
    author="Anne Mulhern",
    author_email="amulhern@redhat.com",
    description="testing library for stratisd",
    long_description=open(README, encoding="utf-8").read(),
    platforms=["Linux"],
    install_requires=["dbus-client-gen>=0.3", "dbus-python-client-gen>=0.6"],
    package_dir={"": "src"},
    packages=setuptools.find_packages("src"),
)
