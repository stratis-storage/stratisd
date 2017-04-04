// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::prelude::AsRawFd;
use std::path::Path;
use std::str::FromStr;

use devicemapper::Device;
use devicemapper::consts::SECTOR_SIZE;
use devicemapper::types::{Bytes, Sectors};

use engine::{EngineResult, EngineError};

ioctl!(read blkgetsize64 with 0x12, 114; u64);

pub fn blkdev_size(file: &File) -> EngineResult<Bytes> {
    let mut val: u64 = 0;

    match unsafe { blkgetsize64(file.as_raw_fd(), &mut val) } {
        Err(x) => Err(EngineError::Nix(x)),
        Ok(_) => Ok(Bytes(val)),
    }
}

/// Resolve a list of Paths of some sort to a set of unique Devices.
/// Return an IOError if there was a problem resolving any particular device.
pub fn resolve_devices(paths: &[&Path]) -> io::Result<HashSet<Device>> {
    let mut devices = HashSet::new();
    for path in paths {
        let dev = try!(Device::from_str(&path.to_string_lossy()));
        devices.insert(dev);
    }
    Ok(devices)
}

/// Write buf at offset length times.
pub fn write_sectors(path: &Path,
                     offset: Sectors,
                     length: Sectors,
                     buf: &[u8; SECTOR_SIZE])
                     -> EngineResult<()> {
    let mut f = try!(OpenOptions::new().write(true).open(path));
    // set the start point to the offset
    try!(f.seek(SeekFrom::Start(*offset)));
    for _ in 0..*length {
        try!(f.write_all(buf));
    }
    try!(f.flush());
    Ok(())
}


/// Zero sectors at the given offset for length sectors.
pub fn wipe_sectors(path: &Path, offset: Sectors, length: Sectors) -> EngineResult<()> {
    write_sectors(path, offset, length, &[0u8; SECTOR_SIZE])
}
