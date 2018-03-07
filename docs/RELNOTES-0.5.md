# Release notes for Stratis 0.5 (March 8, 2018)

This release is suitable for developers and early testers. It should not be
used with valuable data, and pools created with this release will not be
supported in Stratis 1.0, due to upcoming on-disk format changes.

## New Features

* Snapshots. In addition to being created from scratch (with `fs create`), a
  new filesystem may now be created as a read/write snapshot of an existing
  filesystem, using the `fs snapshot` command.
* Cache tier. Using the `blockdev add-cache` command, a high-performing
  blockdev, such as an SSD, may be added to a pool to act as a cache for the
  regular data tier. The existing `blockdev add` command has been renamed
  `blockdev add-data`.
* Event-driven. stratisd now uses the new device-mapper (DM) event mechanism,
  instead of polling its devices every ten seconds. stratisd also expands a
  pool's thinpool metadata and data devices based upon the lowater threshold
  event.
* Devices under /dev. Stratis now represents its pools and their filesystems
  under `/dev/stratis`, making it easier to mount and use them.
* Thin Check. When activating a pool, Stratis will now automatically run
  `thin_check`, and if needed, `thin_repair`.
* Block devices that make up a pool are now exposed via the D-Bus API, as well
  as the `blockdev list` command.
* Udev integration. Stratis will now track incomplete pools, and use udev
  device-added notifications to complete and activate them, if added later.
  
## Known issues

* It is currently only possible for the `fs create` command to create one
  filesystem at a time. [(issue)](https://github.com/stratis-storage/stratisd/issues/694)
* Automatic management of filesystem size is not working. [(issue)](https://github.com/stratis-storage/stratisd/issues/695)
  
