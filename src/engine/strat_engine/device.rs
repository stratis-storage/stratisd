// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::fs::File;
use std::io::{Seek, Write, SeekFrom};
use std::fs::OpenOptions;
use std::os::unix::prelude::AsRawFd;
use std::path::Path;

use devicemapper::consts::SECTOR_SIZE;
use devicemapper::{Bytes, Sectors};

use engine::{EngineResult, EngineError};

ioctl!(read blkgetsize64 with 0x12, 114; u64);

pub fn blkdev_size(file: &File) -> EngineResult<Bytes> {
    let mut val: u64 = 0;

    match unsafe { blkgetsize64(file.as_raw_fd(), &mut val) } {
        Err(x) => Err(EngineError::Nix(x)),
        Ok(_) => Ok(Bytes(val)),
    }
}

/// Write buf at offset length times.
pub fn write_sectors(path: &Path,
                     offset: Sectors,
                     length: Sectors,
                     buf: &[u8; SECTOR_SIZE])
                     -> EngineResult<()> {
    let mut f = try!(OpenOptions::new().write(true).open(path));

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
