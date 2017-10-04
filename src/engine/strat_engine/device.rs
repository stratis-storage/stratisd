// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, ErrorKind, Seek, SeekFrom, Write};
use std::fs::OpenOptions;
use std::os::linux::fs::MetadataExt;
use std::os::unix::prelude::AsRawFd;
use std::path::Path;

use nix::sys::stat::{S_IFBLK, S_IFMT};

use devicemapper::{Bytes, Device, IEC, SECTOR_SIZE, Sectors};

use super::super::errors::{EngineResult, EngineError, ErrorEnum};

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
    let mut f = BufWriter::with_capacity(IEC::Mi as usize,
                                         OpenOptions::new().write(true).open(path)?);

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
        Err(err) => {
            if err.kind() == ErrorKind::NotFound {
                return Ok(None);
            }
            Err(err)?
        }
    }
}


/// Resolve a list of Paths of some sort to a set of unique Devices.
/// Return an IOError if there was a problem resolving any particular device.
/// The set of devices maps each device to one of the paths passed.
/// Returns an error if any path does not correspond to a block device.
pub fn resolve_devices<'a>(paths: &'a [&Path]) -> EngineResult<HashMap<Device, &'a Path>> {
    let mut map = HashMap::new();
    for path in paths {
        match devnode_to_devno(path)? {
            Some(devno) => {
                let _ = map.insert(Device::from(devno), *path);
            }
            None => {
                let err_msg = format!("path {} does not refer to a block device", path.display());
                return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        }
    }
    Ok(map)
}
