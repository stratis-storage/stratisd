# stratisd

A daemon that manages a pool of block devices to create flexible filesystems.

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

#### Development Compiler
The version of the compiler recommended for development is 1.41. Other
versions of the compiler may disagree with the CI tasks on some points,
so should be avoided.

#### Building
Stratisd requires Rust 1.41+ and Cargo to build. These may be available via
your distribution's package manager. If not, [Rustup](https://www.rustup.rs/)
is available to install and update the Rust toolchain.
Once toolchain and other dependencies are in place, run `make build` to build, and then run the
`stratisd` executable in `./target/x86_64-unknown-linux-gnu/debug` as root.
Pass the `--help` option for more information on additional developer options.

##### Building tests
The Makefile provides a target, `build-tests` which allows compiling the
tests without running any of them, as a convenience to developers.

##### Secondary dependencies
The rust library dbus-rs has an external dependency on the C dbus library
[dbus development library](https://www.freedesktop.org/wiki/Software/dbus/).
Please check with your distributions package manager to locate the needed
package (e.g. `libdbus-1-dev` for Debian-based, `dbus-devel` for Fedora RPM-based
Linux distributions).

The files needed to build dbus-rs include, but are not limited to:

```
/usr/include/dbus-1.0/dbus/dbus*.h
/usr/lib64/libdbus-1.so
/usr/lib64/pkgconfig/dbus-1.pc
```

Also, the rust library libudev-sys has a dependency on the C libudev library.
Please check with your distributions package manager to locate the needed
package (e.g. `libudev-dev` for Debian-based, `systemd-devel` for Fedora RPM-based
Linux distributions).

At least, you need to include:

```
/usr/lib64/pkgconfig/libudev.pc
```


#### Formatting
Stratisd makes use of `rustfmt` to enforce consistent formatting in Rust
files.  PRs must pass the `fmt` task in the CI in order to be merged.
Run `make fmt` to ensure your changes conform to the expected formatting
before submitting a pull request. Formatting changes a bit with different
versions of the compiler; make sure to use the current development version.

#### Linting
Stratisd makes use of `clippy` to detect Rust lints. PRs must pass the
`clippy` task in the CI in order to be merged. To check for lints, run
`make clippy`. The lints change a bit with different versions of the compiler;
make sure to use the current development version.

#### Configuring

Stratisd runs as root, and requires access to the D-Bus system bus. Thus in
order to work properly, a D-Bus conf file must exist to grant access, either
installed by distribution packaging; or manually, by copying `stratisd.conf`
to `/etc/dbus-1/system.d/`.

#### Testing

Stratisd is tested in two ways. The first way makes use of the Rust test
infrastructure and has more access to stratisd internals. The second way
makes use of the stratisd D-Bus interface.

##### Tests that make use of the Rust test infrastructure
Stratisd incorporates two testing modalities:
* safe unit tests, which can be run without affecting your storage configuration
* unsafe unit tests, which may create and destroy devices during execution

To run the safe unit tests:

```bash
$ make test
```

For a description of the unsafe unit tests, necessary setup steps, and how to
run them, see [`tests/README.md`](tests/README.md).

##### Test that interact with stratisd via the D-Bus
For a description of the D-Bus-based tests see
[`tests/client-dbus/README.rst`](tests/client-dbus/README.rst).

## Licensing

[MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/). All
contributions retain ownership by their original author, but must also be
licensed under the MPL 2.0 to be merged.
