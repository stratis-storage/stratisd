// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::fs::File;
use std::io::{Seek, Write, SeekFrom};
use std::fs::OpenOptions;
use std::os::linux::fs::MetadataExt;
use std::os::unix::prelude::AsRawFd;
use std::path::Path;

use nix::sys::stat::{S_IFBLK, S_IFMT};

use devicemapper::consts::SECTOR_SIZE;
use devicemapper::{Bytes, Sectors};

use super::super::errors::{EngineResult, EngineError};

ioctl!(read blkgetsize64 with 0x12, 114; u64);

pub fn blkdev_size(file: &File) -> EngineResult<Bytes> {
    let mut val: u64 = 0;

    match unsafe { blkgetsize64(file.as_raw_fd(), &mut val) } {
        Err(x) => Err(EngineError::Nix(x)),
        Ok(_) => Ok(Bytes(val)),
    }
}

/// Write buf at offset length times.
pub fn write_sectors<P: AsRef<Path>>(path: P,
                                     offset: Sectors,
                                     length: Sectors,
                                     buf: &[u8; SECTOR_SIZE])
                                     -> EngineResult<()> {
    let mut f = OpenOptions::new().write(true).open(path)?;

    f.seek(SeekFrom::Start(*offset))?;
    for _ in 0..*length {
        f.write_all(buf)?;
    }

    f.flush()?;
    Ok(())
}

/// Zero sectors at the given offset for length sectors.
pub fn wipe_sectors<P: AsRef<Path>>(path: P, offset: Sectors, length: Sectors) -> EngineResult<()> {
    write_sectors(path, offset, length, &[0u8; SECTOR_SIZE])
}

/// Get a device number from a device node.
/// Return None if the device is not a block device; devicemapper is not
/// interested in other sorts of devices.
pub fn devnode_to_devno(path: &Path) -> EngineResult<Option<u64>> {
    match path.metadata() {
        Ok(metadata) => {
            Ok(if metadata.st_mode() & S_IFMT.bits() == S_IFBLK.bits() {
                   Some(metadata.st_rdev())
               } else {
                   None
               })
        }
        Err(err) => Err(err)?,
    }
}
