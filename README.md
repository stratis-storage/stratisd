# stratisd

A daemon that manages a pool of block devices to create flexible filesystems.

## Status

September 28 2018: Stratis 1.0 released.
See [release notes](https://github.com/stratis-storage/stratis-docs/blob/master/docs/relnotes/relnotes-1.0.md) for details.

## Background

Stratis (which includes [stratisd](https://github.com/stratis-storage/stratisd)
as well as [stratis-cli](https://github.com/stratis-storage/stratis-cli)),
provides ZFS/Btrfs-style features by integrating layers of existing technology:
Linux's devicemapper subsystem, and the XFS filesystem. `stratisd` manages
collections of block devices, and exports a D-Bus API. Stratis-cli's `stratis`
provides a command-line tool which itself uses the D-Bus API to communicate
with `stratisd`.

## Documentation

https://stratis-storage.github.io/ currently has links to the
[main internal design doc](https://stratis-storage.github.io/StratisSoftwareDesign.pdf),
the [D-Bus API Reference manual](https://stratis-storage.github.io/DBusAPIReference.pdf),
and [some coding style guidelines](https://stratis-storage.github.io/StratisStyleGuidelines.pdf).

## Getting involved

### Communication channels

If you have questions, please don't hesitate to ask them, either on the mailing list or
IRC! :smiley:

#### Mailing list

Development mailing list: stratis-devel@lists.fedorahosted.org, -- subscribe
[here](https://lists.fedoraproject.org/admin/lists/stratis-devel.lists.fedorahosted.org/).

#### IRC

irc.freenode.net #stratis-storage.

## For Developers

Stratisd is written in [Rust](https://www.rust-lang.org), which helps the
implementation be small, correct, and avoid requiring shipping with a large
language runtime.

### Issue tracking and Development

Stratisd development uses GitHub issue tracking, and new development occurs via
GitHub pull requests (PRs). Patches or bug reports may also be sent to the
mailing list, if preferred.

### Setting up for development

#### Dbus configuration file

Stratisd runs as root, and requires access to the D-Bus system bus. Thus in
order to work properly, a D-Bus conf file must exist to grant access, either
installed by distribution packaging; or manually, by copying `stratisd.conf`
to `/etc/dbus-1/system.d/`.


#### Rust tools
Stratisd requires Rust 1.31+ and Cargo to build. These may be available via
your distribution's package manager. If not, [Rustup](https://www.rustup.rs/)
is available to install and update the Rust toolchain.

Stratisd makes use of `rustfmt` to enforce consistent formatting in Rust
files.  PRs must pass the `fmt` task in the CI in order to be merged. The
`fmt` task currently uses rustfmt 1.0 as shipped with Rust 1.31).

#### Secondary dependencies
The rust library dbus-rs has an external dependency on the C dbus library
[dbus development library](https://www.freedesktop.org/wiki/Software/dbus/).
Please check with your distributions package manager to locate the needed
package.

The files needed to build dbus-rs include, but are not limited to:

```
/usr/include/dbus-1.0/dbus/dbus*.h
/usr/lib64/libdbus-1.so
/usr/lib64/pkgconfig/dbus-1.pc
```

Also, the rust library libudev-sys has an dependency on the C libudev library.
Please check with your distributions package manager to locate the needed
package (e.g libudev-dev for Debian-based, systemd-devel for Fedora RPM-based
Linux distributions).

At least, you need to include:

```
/usr/lib64/pkgconfig/libudev.pc
```

#### Building
Once toolchain and other dependencies are in place, run `cargo build` to build, and then run the
`stratisd` executable in `./target/debug/` as root. Pass the `--help` option
for more information on additional developer options.

#### Reformatting
To reformat all files to ensure proper formatting, run `cargo fmt` to ensure
your changes conform to the expected formatting before submitting a pull request.

#### Testing
Stratisd incorporates two testing modalities:
* safe unit tests, which can be run without affecting your storage configuration
* unsafe unit tests, which may create and destroy devices during execution

To run the safe unit tests:

```bash
$ make test
```

For a description of the unsafe unit tests, necessary setup steps, and how to
run them, see [`tests/README.md`](tests/README.md).

## Licensing

[MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/). All
contributions retain ownership by their original author, but must also be
licensed under the MPL 2.0 to be merged.
