// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::fs::OpenOptions;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use devicemapper::{IEC, SECTOR_SIZE, Sectors};

use stratis::StratisResult;

/// Write buf at offset length times.
pub fn write_sectors<P: AsRef<Path>>(path: P,
                                     offset: Sectors,
                                     length: Sectors,
                                     buf: &[u8; SECTOR_SIZE])
                                     -> StratisResult<()> {
    let mut f = BufWriter::with_capacity(IEC::Mi as usize,
                                         OpenOptions::new().write(true).open(path)?);

    f.seek(SeekFrom::Start(*offset.bytes()))?;
    for _ in 0..*length {
        f.write_all(buf)?;
    }

    f.flush()?;
    Ok(())
}

/// Zero sectors at the given offset for length sectors.
pub fn wipe_sectors<P: AsRef<Path>>(path: P,
                                    offset: Sectors,
                                    length: Sectors)
                                    -> StratisResult<()> {
    write_sectors(path, offset, length, &[0u8; SECTOR_SIZE])
}
