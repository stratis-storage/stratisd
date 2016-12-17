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

use super::consts::MIN_MDA_SIZE;

const MDA_OFFSETS: [usize; 4] = [8, 12, 16, 20];
const NUM_MDA_COPIES: u64 = 2;

#[derive(Debug, Clone, Copy)]
pub struct MDA {
    // Recorded values
    pub crc: u32,
    pub last_updated: Timespec,
    pub used: u32,

    // Computed values
    pub offset: SectorOffset, // From start of MDA, not BDA
}

impl MDA {
    pub fn new(offset: SectorOffset) -> MDA {
        MDA {
            crc: 0,
            last_updated: Timespec::new(0, 0),
            used: 0,
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
    pub fn read(buf: &[u8], buf_offset: usize, offset: SectorOffset) -> MDA {
        let (offset1, offset2, offset3, offset4) = MDA::offsets(buf_offset);

        MDA {
            last_updated: Timespec::new(LittleEndian::read_u64(&buf[buf_offset..offset1]) as i64,
                                        LittleEndian::read_u32(&buf[offset1..offset2]) as i32),
            used: LittleEndian::read_u32(&buf[offset2..offset3]),
            crc: LittleEndian::read_u32(&buf[offset3..offset4]),
            offset: offset,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MDAGroup {
    pub mda_length: u32,
    mdaa: MDA,
    mdab: MDA,
}

impl MDAGroup {
    pub fn new(size: Sectors) -> MDAGroup {
        MDAGroup {
            mda_length: ((*size / NUM_MDA_COPIES) * SECTOR_SIZE) as u32,
            mdaa: MDA::new(SectorOffset(0)),
            mdab: MDA::new(SectorOffset(*size / NUM_MDA_COPIES)),
        }
    }

    /// Read MDAGroup from buf at offset.
    /// Use size to calculate values for each MDA in the group.
    pub fn read(buf: &[u8], offset: usize, size: Sectors) -> MDAGroup {
        MDAGroup {
            mda_length: ((*size / NUM_MDA_COPIES) * SECTOR_SIZE) as u32,
            mdaa: MDA::read(&buf, offset, SectorOffset(0)),
            mdab: MDA::read(&buf, offset + 32, SectorOffset(*size / NUM_MDA_COPIES)),
        }
    }

    /// Write MDAGroup to buf at offset.
    pub fn write(&self, buf: &mut [u8], offset: usize) {
        self.mdaa.write(buf, offset);
        self.mdab.write(buf, offset + 32);
    }

    /// Return the most recently timestamped MDA.
    pub fn most_recent(&self) -> &MDA {
        match self.mdaa.last_updated.cmp(&self.mdab.last_updated) {
            Ordering::Less => &self.mdab,
            Ordering::Greater => &self.mdaa,
            _ => &self.mdab,
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


/// Validate MDA size
/// Return None if MDA size is fine, otherwise a message.
pub fn validate_mda_size(size: Sectors) -> Option<String> {
    if *size % NUM_MDA_COPIES != 0 {
        let error_message = format!("MDA size {} is not divisible by number of copies \
                                    required {}",
                                    *size,
                                    NUM_MDA_COPIES);
        return Some(error_message);
    };

    if size < MIN_MDA_SIZE {
        let error_message = format!("MDA size {} is less than minimum ({})",
                                    *size,
                                    *MIN_MDA_SIZE);
        return Some(error_message);
    };
    None
}
