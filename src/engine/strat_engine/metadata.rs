// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cmp::Ordering;
use std::str::from_utf8;

use byteorder::ByteOrder;
use byteorder::LittleEndian;

use crc::crc32;

use time::Timespec;

use uuid::Uuid;

use consts::SECTOR_SIZE;

use types::Sectors;
use types::SectorOffset;

use super::consts::MDA_RESERVED_SIZE;
use super::consts::MIN_MDA_SIZE;
use super::consts::NUM_MDA_COPIES;
use super::consts::STRAT_MAGIC;

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

#[derive(Debug, Clone)]
pub struct SigBlock {
    pub total_size: Sectors,
    pub pool_uuid: Uuid,
    pub dev_uuid: Uuid,
    pub mda: MDAGroup,
    pub mda_sectors: Sectors,
    pub reserved_sectors: Sectors,
}

impl SigBlock {
    pub fn new(pool_uuid: &Uuid,
               dev_uuid: &Uuid,
               mda_size: Sectors,
               total_size: Sectors)
               -> SigBlock {
        SigBlock {
            mda: MDAGroup::new(mda_size),
            pool_uuid: pool_uuid.clone(),
            dev_uuid: dev_uuid.clone(),
            total_size: total_size,
            mda_sectors: mda_size,
            reserved_sectors: MDA_RESERVED_SIZE,
        }
    }

    /// Write SigBlock to buf at offset.
    pub fn write(&self, buf: &mut [u8], offset: usize) {
        buf[offset + 4..offset + 20].clone_from_slice(STRAT_MAGIC);
        LittleEndian::write_u64(&mut buf[offset + 20..offset + 28], *self.total_size);
        buf[offset + 32..offset + 64]
            .clone_from_slice(self.pool_uuid.simple().to_string().as_bytes());
        buf[offset + 64..offset + 96]
            .clone_from_slice(self.dev_uuid.simple().to_string().as_bytes());
        self.mda.write(buf, offset + 96);
        LittleEndian::write_u32(&mut buf[offset + 160..offset + 164],
                                *self.mda_sectors as u32);
        LittleEndian::write_u32(&mut buf[offset + 164..offset + 168],
                                *self.reserved_sectors as u32);

        let hdr_crc = crc32::checksum_ieee(&buf[offset + 4..offset + SECTOR_SIZE as usize]);
        LittleEndian::write_u32(&mut buf[offset..offset + 4], hdr_crc);
    }

    /// Read SigBlock from buf at offset.
    pub fn read(buf: &[u8], offset: usize, total_size: Sectors) -> Result<SigBlock, String> {
        if &buf[offset + 4..offset + 20] != STRAT_MAGIC {
            return Err("not a Stratis device".into());
        }

        let crc = crc32::checksum_ieee(&buf[offset + 4..offset + SECTOR_SIZE as usize]);
        if crc != LittleEndian::read_u32(&buf[offset..offset + 4]) {
            return Err("header CRC invalid".into());
        }

        let pool_uuid = try!(Uuid::parse_str(from_utf8(&buf[offset + 32..offset + 64]).unwrap())
            .map_err(|_| "invalid pool uuid"));
        let dev_uuid = try!(Uuid::parse_str(from_utf8(&buf[offset + 64..offset + 96]).unwrap())
            .map_err(|_| "invalid dev uuid"));

        let mda_size = Sectors(LittleEndian::read_u32(&buf[offset + 160..offset + 164]) as u64);

        match validate_mda_size(mda_size) {
            None => {}
            Some(err) => {
                return Err(err.into());
            }
        };

        Ok(SigBlock {
            mda: MDAGroup::read(buf, offset + 96, mda_size),
            pool_uuid: pool_uuid,
            dev_uuid: dev_uuid,
            total_size: total_size,
            mda_sectors: mda_size,
            reserved_sectors:
                Sectors(LittleEndian::read_u32(&buf[offset + 164..offset + 168]) as u64),
        })
    }

    /// Read from Stratis magic number area in buf at offset.
    pub fn read_strat_magic(buf: &[u8], offset: usize) -> &[u8] {
        &buf[offset + 4..offset + 20]
    }

    /// Read pool uuid from buf at offset.
    /// Return an error if the data can't be interpreted as a Uuid.
    pub fn read_pool_uuid(buf: &[u8], offset: usize) -> Result<Uuid, String> {
        Uuid::parse_str(from_utf8(&buf[offset + 32..offset + 64]).unwrap())
            .map_err(|_| "invalid pool uuid".into())
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
