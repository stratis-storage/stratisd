#!/bin/bash

curl -sSf https://build.travis-ci.org/files/rustup-init.sh | sh -s -- --default-toolchain=stable -y || exit 1
curl -sSf https://build.travis-ci.org/files/rustup-init.sh | sh -s -- --default-toolchain=nightly -y || exit 1

dnf -y install gcc dbus-devel make xfsprogs sudo || exit 1

source ~/.cargo/env || exit 1
rustup default stable || exit 1

cd /stratisd-code || exit 1

make fmt || exit 1
make build || exit 1
make docs || exit 1
make test || exit 1
make test-loop || exit 1
# run clippy with nightly
rustup default nightly
make clippy || exit 1
