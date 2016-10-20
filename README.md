# stratisd

A daemon that manages a pool of block devices to create flexible filesystems.

## Background

There are many benefits to volume-managing filesystems (VMFs) like ZFS and Btrfs. In contrast to traditional Unix filesystems, VMFs can span multiple block devices, and support multiple independent filesystem trees. (ZFS calls these datasets, Btrfs calls these subvolumes.)  VMFs can share space between trees using copy-on-write, and support using their multiple block devices to provide RAID-style protection from data loss.

Stratis (which includes [stratisd](https://github.com/stratis-storage/stratisd) as well as [stratis-cli](https://github.com/stratis-storage/stratis-cli)), attempts to provide VMF-style features by integrating layers of existing technology: Linux's devicemapper subsystem, and the non-VMF, high-performance XFS filesystem. `stratisd` manages collections of block devices, and exports a D-Bus API. Stratis-cli's `stratis` provides a command-line tool which itself uses the D-Bus API to communicate with `stratisd`.

## Implementation

Stratisd is written in [Rust](https://www.rust-lang.org), which helps the implementation be small, correct, and not require a large language runtime.

## Documentation

Please see https://stratis-storage.github.io/ and [our documentation repo](https://github.com/stratis-storage/stratis-docs).

## Contributing

Stratisd development uses GitHub tools for development and issue tracking. We don't have mailing lists yet, so please feel free to open an issue, even for a question.

It is licensed under the [MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/). All contributions retain ownership by their original author, but must also be licensed under the MPL 2.0 to be merged by us.
