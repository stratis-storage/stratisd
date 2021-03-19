#!/bin/sh

cargo update -p clap --precise 2.21.1
cargo update -p nix:0.20.0 --precise 0.20.0
cargo update -p devicemapper --precise 0.29.0
cargo update -p crc --precise 1.0.0
cargo update -p byteorder --precise 1.2.3
cargo update -p chrono --precise 0.4.1
(cargo update -p rand:0.8.3 --precise 0.8.0 >& /dev/null || cargo update -p rand:0.8.0 --precise 0.8.0)
cargo update -p serde --precise 1.0.119
cargo update -p serde_derive --precise 1.0.119
cargo update -p serde_json --precise 1.0.50
cargo update -p tempfile --precise 3.0.2
cargo update -p log --precise 0.4.8
(cargo update -p env_logger:0.8.3 --precise 0.8.0 >& /dev/null || cargo update -p env_logger:0.8.0 --precise 0.8.0)
cargo update -p libc --precise 0.2.86
cargo update -p libmount --precise 0.1.9
cargo update -p libudev --precise 0.2.0
(cargo update -p lazy_static:1.4.0 --precise 1.2.0 >& /dev/null || cargo update -p lazy_static:1.2.0 --precise 1.2.0)
cargo update -p timerfd --precise 1.0.0
cargo update -p itertools --precise 0.10.0
cargo update -p semver:0.11.0 --precise 0.11.0
cargo update -p termios --precise 0.3.0
(cargo update -p regex:1.4.5 --precise 1.4.0 >& /dev/null || cargo update -p regex:1.4.0 --precise 1.4.0)
cargo update -p base64 --precise 0.13.0
cargo update -p sha-1 --precise 0.9.0
cargo update -p either --precise 1.5.0
cargo update -p futures --precise 0.3.5
cargo update -p libcryptsetup-rs --precise 0.4.3
cargo update -p tokio --precise 1.2.0
cargo update -p dbus --precise 0.9.0
cargo update -p dbus-tree --precise 0.9.0
cargo update -p dbus-tokio --precise 0.7.0
cargo update -p libdbus-sys --precise 0.2.1
cargo update -p uuid --precise 0.8.0
cargo update -p pkg-config --precise 0.3.17
cargo update -p error-chain --precise 0.12.4
cargo update -p loopdev --precise 0.2.0
cargo update -p proptest --precise 0.10.0
cargo update -p matches --precise 0.1.3
