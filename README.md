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

### Status

Stratis is currently in early stages of development and is a few months away from being
ready for initial testing by users.

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
GitHub pull requests (PRs).

### Setting up for development

#### Dbus configuration file

Stratisd runs as root, and requires access to the D-Bus system bus. Thus in
order to work properly, a D-Bus conf file must exist to grant access, either
installed by distribution packaging; or manually, by copying `stratisd.conf`
to `/etc/dbus-1/system.d/`.


#### Rust tools
Stratisd requires Rust 1.17+ and Cargo to build. These may be available via
your distribution's package manager. If not, [Rustup](https://www.rustup.rs/)
is available to install and update the Rust toolchain.

Stratisd makes use of `rustfmt` to enforce consistent formatting in Rust files.
PRs must pass the `rustfmt` task in the CI in order to be merged.
The `rustfmt` task currently requires the specific `rustfmt` version 0.8.3.
Installation of this specific version can be achieved via:

```
cargo install --vers 0.8.3 rustfmt
```

#### Secondary dependencies
The rust library dbus-rs has an external dependency on the C dbus library
[dbus development library](https://www.freedesktop.org/wiki/Software/dbus/).
Please check with your distributions package manager to locate the needed
package.

The files needed to build dbus-rs include, but are not limited too:

```
/usr/include/dbus-1.0/dbus/dbus*.h
/usr/lib64/libdbus-1.so
/usr/lib64/pkgconfig/dbus-1.pc
```

#### Building
Once toolchain and other dependencies are in place, run `cargo build` to build, and then run the
`stratisd` executable in `./target/debug/` as root. Pass the `--help` option
for more information on additional developer options.

#### Reformatting
To reformat all files to ensure proper formatting, run `cargo fmt` to ensure
your changes conform to the expected formatting before submitting a pull request.

#### Testing
Stratisd incorporates two testing modalities: unit tests, which are defined
in the source code, and integration tests which can be found in a separate
tests directory. To run the unit tests:

```bash
$ make test
```

A description of the integration tests can be found in the tests directory.

## Licensing

[MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/). All
contributions retain ownership by their original author, but must also be
licensed under the MPL 2.0 to be merged.
