// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Functions to unify writing to devices

#[cfg(test)]
use std::io::Cursor;
use std::{
    cmp::min,
    fs::{File, OpenOptions},
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::Path,
};

use devicemapper::{Sectors, IEC, SECTOR_SIZE};

use crate::stratis::StratisResult;

/// The SyncAll trait unifies the File type with other types that do
/// not implement sync_all().
pub trait SyncAll: Write {
    fn sync_all(&mut self) -> io::Result<()>;
}

impl SyncAll for File {
    /// Invokes File::sync_all() thereby syncing all the data
    fn sync_all(&mut self) -> io::Result<()> {
        File::sync_all(self)
    }
}

#[cfg(test)]
impl<T> SyncAll for Cursor<T>
where
    Cursor<T>: Write,
{
    /// A no-op. No data need be synced, because it is already in the Cursor
    /// inner value, which has type T.
    fn sync_all(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<T> SyncAll for BufWriter<T>
where
    T: SyncAll,
{
    fn sync_all(&mut self) -> io::Result<()> {
        self.flush()?;
        self.get_mut().sync_all()
    }
}

/// Write buf at offset length times.
fn write_sectors<P: AsRef<Path>>(
    path: P,
    offset: Sectors,
    length: Sectors,
    buf: &[u8; SECTOR_SIZE],
) -> StratisResult<()> {
    let mut f = BufWriter::with_capacity(
        convert_const!(min(u128::from(IEC::Mi), *(length.bytes())), u128, usize),
        OpenOptions::new().write(true).open(path)?,
    );

    f.seek(SeekFrom::Start(convert_int!(*offset.bytes(), u128, u64)?))?;
    for _ in 0..*length {
        f.write_all(buf)?;
    }

    f.sync_all()?;
    Ok(())
}

/// Zero sectors at the given offset for length sectors.
/// Note that this method buffers the zeros and syncs only when all are
/// written.
pub fn wipe_sectors<P: AsRef<Path>>(
    path: P,
    offset: Sectors,
    length: Sectors,
) -> StratisResult<()> {
    write_sectors(path, offset, length, &[0u8; SECTOR_SIZE])
}
