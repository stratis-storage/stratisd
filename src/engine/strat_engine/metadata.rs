// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cmp::Ordering;

use byteorder::ByteOrder;
use byteorder::LittleEndian;
use time::Timespec;

use consts::SECTOR_SIZE;

use types::Sectors;
use types::SectorOffset;

use super::consts::NUM_MDA_COPIES;

#[derive(Debug, Clone, Copy)]
pub struct MDA {
    // Recorded values
    pub crc: u32,
    pub last_updated: Timespec,
    pub used: u32,

    // Computed values
    pub length: u32,
    pub offset: SectorOffset, // From start of MDA, not BDA
}

const MDA_OFFSETS: [usize; 4] = [8, 12, 16, 20];

impl MDA {
    pub fn new(length: u32, offset: SectorOffset) -> MDA {
        MDA {
            crc: 0,
            last_updated: Timespec::new(0, 0),
            used: 0,
            length: length,
            offset: offset,
        }
    }

    fn offsets(offset: usize) -> (usize, usize, usize, usize) {
        (offset + MDA_OFFSETS[0],
         offset + MDA_OFFSETS[1],
         offset + MDA_OFFSETS[2],
         offset + MDA_OFFSETS[3])
    }

    /// Write out recorded values to buf at offset.
    pub fn write(&self, buf: &mut [u8], offset: usize) {
        let (offset1, offset2, offset3, offset4) = MDA::offsets(offset);

        let last_updated = self.last_updated;
        LittleEndian::write_u64(&mut buf[offset..offset1], last_updated.sec as u64);
        LittleEndian::write_u32(&mut buf[offset1..offset2], last_updated.nsec as u32);

        LittleEndian::write_u32(&mut buf[offset2..offset3], self.used);
        LittleEndian::write_u32(&mut buf[offset3..offset4], self.crc);
    }

    /// Read recorded values into buffer at buf_offset.
    pub fn read(buf: &[u8], buf_offset: usize, length: u32, offset: SectorOffset) -> MDA {
        let (offset1, offset2, offset3, offset4) = MDA::offsets(buf_offset);

        MDA {
            last_updated: Timespec::new(LittleEndian::read_u64(&buf[buf_offset..offset1]) as i64,
                                        LittleEndian::read_u32(&buf[offset1..offset2]) as i32),
            used: LittleEndian::read_u32(&buf[offset2..offset3]),
            crc: LittleEndian::read_u32(&buf[offset3..offset4]),
            length: length,
            offset: offset,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MDAGroup {
    mdaa: MDA,
    mdab: MDA,
}

impl MDAGroup {
    pub fn new(size: Sectors) -> MDAGroup {
        let length = ((*size / NUM_MDA_COPIES) * SECTOR_SIZE) as u32;
        MDAGroup {
            mdaa: MDA::new(length, SectorOffset(0)),
            mdab: MDA::new(length, SectorOffset(*size / NUM_MDA_COPIES)),
        }
    }

    /// Read MDAGroup from buf at offset.
    /// Use size to calculate values for each MDA in the group.
    pub fn read(buf: &[u8], offset: usize, size: Sectors) -> MDAGroup {
        let length = ((*size / NUM_MDA_COPIES) * SECTOR_SIZE) as u32;
        MDAGroup {
            mdaa: MDA::read(&buf, offset, length, SectorOffset(0)),
            mdab: MDA::read(&buf,
                            offset + 32,
                            length,
                            SectorOffset(*size / NUM_MDA_COPIES)),
        }
    }

    /// Write MDAGroup to buf at offset.
    pub fn write(&self, buf: &mut [u8], offset: usize) {
        self.mdaa.write(buf, offset);
        self.mdab.write(buf, offset + 32);
    }

    /// Return the most recently timestamped MDA.
    /// Return None if neither has a non-zero timestamp.
    pub fn most_recent(&self) -> Option<&MDA> {
        let mda = match self.mdaa.last_updated.cmp(&self.mdab.last_updated) {
            Ordering::Less => &self.mdab,
            Ordering::Greater => &self.mdaa,
            _ => &self.mdab,
        };
        if mda.last_updated == Timespec::new(0, 0) {
            None
        } else {
            Some(mda)
        }
    }

    /// Return the least recently timestamped MDA.
    pub fn least_recent(&mut self) -> &mut MDA {
        match self.mdaa.last_updated.cmp(&self.mdab.last_updated) {
            Ordering::Less => &mut self.mdaa,
            Ordering::Greater => &mut self.mdab,
            _ => &mut self.mdaa,
        }
    }
}
