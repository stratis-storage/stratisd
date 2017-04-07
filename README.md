# stratisd

A daemon that manages a pool of block devices to create flexible filesystems.

## Background

There are many benefits to volume-managing filesystems (VMFs) like ZFS and
Btrfs. In contrast to traditional Unix filesystems, VMFs can span multiple block
devices, and support multiple independent filesystem trees. (ZFS calls these
datasets, Btrfs calls these subvolumes.)  VMFs can share space between trees
using copy-on-write, and support using their multiple block devices to provide
RAID-style protection from data loss.

Stratis (which includes [stratisd](https://github.com/stratis-storage/stratisd)
as well as [stratis-cli](https://github.com/stratis-storage/stratis-cli)),
provides VMF-style features by integrating layers of existing technology:
Linux's devicemapper subsystem, and the non-VMF, high-performance XFS
filesystem. `stratisd` manages collections of block devices, and exports a D-Bus
API. Stratis-cli's `stratis` provides a command-line tool which itself uses the
D-Bus API to communicate with `stratisd`.

## Implementation

Stratisd is written in [Rust](https://www.rust-lang.org), which helps the
implementation be small, correct, and not require a large language runtime.

## Documentation

Please see https://stratis-storage.github.io/ and [our documentation
repo](https://github.com/stratis-storage/stratis-docs).

## Contributing

Stratisd development uses GitHub tools for development and issue tracking. We
don't have mailing lists yet, so please feel free to open an issue, even for a
question.

It is licensed under the [MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/). All
contributions retain ownership by their original author, but must also be
licensed under the MPL 2.0 to be merged by us.

### Setting up for development

Stratisd runs as root, and requires access to the D-Bus system bus. Thus in
order to work properly, a D-Bus conf file must exist to grant access, either
installed by distribution packaging; or manually, by copying `stratisd.conf`
to `/etc/dbus-1/system.d/`.

Stratisd requires Rust 1.15.1+ and Cargo to build. These may be available via
your distribution's package manager. If not, [Rustup](https://www.rustup.rs/)
is available to install and update the Rust toolchain.

Once toolchain is in place, run `cargo build` to build, and then run the
`stratisd` executable in `./target/debug/` as root. Pass the `--help` option
for more information on additional developer options.

### Testing
Stratisd incorporates two testing modalities: unit tests, which are defined
in the source code, and integration tests which can be found in a separate
tests directory. To run the unit tests:

> make test

A description of the integration tests can be found in the tests directory.
