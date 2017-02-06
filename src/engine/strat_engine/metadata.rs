// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std;
use std::cmp::Ordering;
use std::str::from_utf8;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use byteorder::ByteOrder;
use byteorder::LittleEndian;
use crc::crc32;
use time::Timespec;
use uuid::Uuid;

use types::{Bytes, Sectors};
use super::engine::DevOwnership;
use consts::{SECTOR_SIZE, MEGA};
use engine::{EngineResult, EngineError, ErrorEnum};

const _BDA_STATIC_HDR_SECTORS: usize = 16;
pub const BDA_STATIC_HDR_SECTORS: Sectors = Sectors(_BDA_STATIC_HDR_SECTORS as u64);
const BDA_STATIC_HDR_SIZE: usize = _BDA_STATIC_HDR_SECTORS * SECTOR_SIZE;
const MDA_RESERVED_SECTORS: Sectors = Sectors(3 * MEGA / (SECTOR_SIZE as u64)); // = 3 MiB

const NUM_MDA_REGIONS: usize = 4;
const PER_MDA_REGION_COPIES: usize = 2;
const NUM_PRIMARY_MDA_REGIONS: usize = NUM_MDA_REGIONS / PER_MDA_REGION_COPIES;

const MDA_REGION_HDR_SIZE: usize = 32;
pub const MIN_MDA_SECTORS: Sectors = Sectors(2032);

const STRAT_MAGIC: &'static [u8] = b"!Stra0tis\x86\xff\x02^\x41rh";

#[derive(Debug)]
pub struct BDA {
    pub header: StaticHeader,
    pub regions: MDARegions,
}

impl BDA {
    /// Initialize a blockdev with a Stratis BDA.
    pub fn initialize(mut f: &mut File, header: StaticHeader) -> EngineResult<BDA> {
        let zeroed = [0u8; BDA_STATIC_HDR_SIZE];
        let hdr_buf = header.sigblock_to_buf();

        // Write 8K header. Static_Header copies go in sectors 1 and 9.
        try!(f.seek(SeekFrom::Start(0)));
        try!(f.write_all(&zeroed[..SECTOR_SIZE]));
        try!(f.write_all(&hdr_buf));
        try!(f.write_all(&zeroed[SECTOR_SIZE * 7..]));
        try!(f.flush());
        try!(f.write_all(&hdr_buf));
        try!(f.write_all(&zeroed[SECTOR_SIZE * 6..]));
        try!(f.flush());

        let regions = try!(MDARegions::initialize(&header, &mut f));

        Ok(BDA {
            header: header,
            regions: regions,
        })
    }

    pub fn load(f: &mut File, header: StaticHeader) -> EngineResult<BDA> {
        let regions = try!(MDARegions::load(&header, f));

        Ok(BDA {
            header: header,
            regions: regions,
        })
    }

    /// Zero out Static Header on the blockdev. This causes it to no
    /// longer be seen as a Stratis blockdev.
    pub fn wipe(f: &mut File) -> EngineResult<()> {
        let zeroed = [0u8; BDA_STATIC_HDR_SIZE];

        // Wiping Static Header should do it
        try!(f.seek(SeekFrom::Start(0)));
        try!(f.write_all(&zeroed));
        try!(f.flush());
        Ok(())
    }

    /// Save metadata to the disk
    pub fn save_state(&mut self,
                      time: &Timespec,
                      metadata: &[u8],
                      mut f: &mut File)
                      -> EngineResult<()> {
        self.regions.save_state(time, metadata, &mut f)
    }

    /// Read latest metadata from the disk
    pub fn load_state(&self, mut f: &mut File) -> EngineResult<Option<Vec<u8>>> {
        self.regions.load_state(&mut f)
    }

    /// The time when the most recent metadata was written to the BDA,
    /// if any.
    pub fn last_update_time(&self) -> &Option<Timespec> {
        &self.regions.mdas[self.regions.newer()].last_updated
    }
}

#[derive(Debug)]
pub struct StaticHeader {
    pub blkdev_size: Sectors,
    pub pool_uuid: Uuid,
    pub dev_uuid: Uuid,
    pub mda_size: Sectors,
    pub reserved_size: Sectors,
    pub flags: u64,
}

impl StaticHeader {
    pub fn new(pool_uuid: &Uuid,
               dev_uuid: &Uuid,
               mda_size: Sectors,
               blkdev_size: Sectors)
               -> StaticHeader {
        StaticHeader {
            blkdev_size: blkdev_size,
            pool_uuid: pool_uuid.clone(),
            dev_uuid: dev_uuid.clone(),
            mda_size: mda_size,
            reserved_size: MDA_RESERVED_SECTORS,
            flags: 0,
        }
    }

    /// Try to find a valid StaticHeader on a device.
    pub fn setup(f: &mut File) -> EngineResult<StaticHeader> {
        try!(f.seek(SeekFrom::Start(0)));
        let mut buf = [0u8; BDA_STATIC_HDR_SIZE];
        try!(f.read(&mut buf));

        StaticHeader::setup_from_buf(&buf)
    }

    /// Try to find a valid StaticHeader in a buffer.
    pub fn setup_from_buf(buf: &[u8; BDA_STATIC_HDR_SIZE]) -> EngineResult<StaticHeader> {
        let sigblock_spots = [&buf[SECTOR_SIZE..2 * SECTOR_SIZE],
                              &buf[9 * SECTOR_SIZE..10 * SECTOR_SIZE]];

        // Check both copies of sigblock for validity
        for buf in &sigblock_spots {
            match StaticHeader::sigblock_from_buf(buf) {
                Ok(sh) => return Ok(sh),
                Err(_) => continue,
            }
        }

        Err(EngineError::Engine(ErrorEnum::Invalid, "not a Stratis device".into()))
    }

    /// Determine the ownership of a device.
    pub fn determine_ownership(f: &mut File) -> EngineResult<DevOwnership> {
        try!(f.seek(SeekFrom::Start(0)));
        let mut buf = [0u8; BDA_STATIC_HDR_SIZE];
        try!(f.read(&mut buf));

        // Using setup() as a test of ownership sets a high bar. It is
        // not sufficient to have STRAT_MAGIC to be considered "Ours",
        // it must also have correct CRC, no weird stuff in fields,
        // etc!
        match StaticHeader::setup_from_buf(&buf) {
            Ok(sh) => Ok(DevOwnership::Ours(sh.pool_uuid)),
            Err(_) => {
                if buf.iter().any(|x| *x != 0) {
                    Ok(DevOwnership::Theirs)
                } else {
                    Ok(DevOwnership::Unowned)
                }
            }
        }
    }

    /// Generate a buf suitable for writing to blockdev
    pub fn sigblock_to_buf(&self) -> [u8; SECTOR_SIZE] {
        let mut buf = [0u8; SECTOR_SIZE];
        buf[4..20].clone_from_slice(STRAT_MAGIC);
        LittleEndian::write_u64(&mut buf[20..28], *self.blkdev_size);
        buf[32..64].clone_from_slice(self.pool_uuid.simple().to_string().as_bytes());
        buf[64..96].clone_from_slice(self.dev_uuid.simple().to_string().as_bytes());
        LittleEndian::write_u64(&mut buf[96..104], *self.mda_size);
        LittleEndian::write_u64(&mut buf[104..112], *self.reserved_size);

        let hdr_crc = crc32::checksum_ieee(&buf[4..SECTOR_SIZE]);
        LittleEndian::write_u32(&mut buf[..4], hdr_crc);
        buf
    }

    /// Build a StaticHeader from a SECTOR_SIZE buf that was read from
    /// a blockdev.
    pub fn sigblock_from_buf(buf: &[u8]) -> EngineResult<StaticHeader> {

        assert_eq!(buf.len(), SECTOR_SIZE);

        if &buf[4..20] != STRAT_MAGIC {
            return Err(EngineError::Engine(ErrorEnum::Invalid, "No Stratis magic".into()));
        }

        let crc = crc32::checksum_ieee(&buf[4..SECTOR_SIZE]);
        if crc != LittleEndian::read_u32(&buf[..4]) {
            return Err(EngineError::Engine(ErrorEnum::Invalid, "header CRC invalid".into()));
        }

        let blkdev_size = Sectors(LittleEndian::read_u64(&buf[20..28]));

        let pool_uuid = try!(Uuid::parse_str(try!(from_utf8(&buf[32..64]))));
        let dev_uuid = try!(Uuid::parse_str(try!(from_utf8(&buf[64..96]))));

        let mda_size = Sectors(LittleEndian::read_u64(&buf[96..104]));

        try!(validate_mda_size(mda_size));

        Ok(StaticHeader {
            pool_uuid: pool_uuid,
            dev_uuid: dev_uuid,
            blkdev_size: blkdev_size,
            mda_size: mda_size,
            reserved_size: Sectors(LittleEndian::read_u64(&buf[104..112])),
            flags: 0,
        })
    }
}

#[derive(Debug)]
pub struct MDARegions {
    // Spec defines 4 regions, but regions 2 & 3 are duplicates of 0 and 1 respectively
    pub region_size: Sectors,
    pub mdas: [MDAHeader; NUM_PRIMARY_MDA_REGIONS],
}

impl MDARegions {
    pub fn initialize(header: &StaticHeader, f: &mut File) -> EngineResult<MDARegions> {
        let hdr_buf = [0u8; MDA_REGION_HDR_SIZE];

        let region_size = header.mda_size / NUM_MDA_REGIONS;
        let per_region_size = region_size.bytes();
        for region in 0..NUM_MDA_REGIONS {
            try!(f.seek(SeekFrom::Start(BDA_STATIC_HDR_SIZE as u64 + *(per_region_size * region))));
            try!(f.write_all(&hdr_buf));
        }

        try!(f.flush());

        Ok(MDARegions {
            region_size: region_size,
            mdas: [MDAHeader::new(per_region_size), MDAHeader::new(per_region_size)],
        })
    }

    // Construct MDARegions based on on-disk info
    pub fn load(header: &StaticHeader, f: &mut File) -> EngineResult<MDARegions> {
        let region_size = header.mda_size / NUM_MDA_REGIONS;
        let per_region_size = region_size.bytes();

        let mut load_a_region = |region: usize| -> EngineResult<MDAHeader> {
            let mut hdr_buf = [0u8; MDA_REGION_HDR_SIZE];
            let offset = BDA_STATIC_HDR_SIZE as u64 + *(per_region_size * region);

            try!(f.seek(SeekFrom::Start(offset)));
            try!(f.read_exact(&mut hdr_buf));
            let mda = try!(MDAHeader::from_buf(&hdr_buf, per_region_size));

            // Loading checks CRC
            try!(mda.load_region(f));

            Ok(mda)
        };

        let mda0 =
            load_a_region(0).unwrap_or_else(|_| {
                load_a_region(2).unwrap_or_else(|_| MDAHeader::new(per_region_size))
            });
        let mda1 =
            load_a_region(1).unwrap_or_else(|_| {
                load_a_region(3).unwrap_or_else(|_| MDAHeader::new(per_region_size))
            });

        Ok(MDARegions {
            region_size: region_size,
            mdas: [mda0, mda1],
        })
    }

    // Write data to the older region
    pub fn save_state(&mut self, time: &Timespec, data: &[u8], f: &mut File) -> EngineResult<()> {
        let used = data.len();
        let data_crc = crc32::checksum_ieee(data);
        let hdr_buf = MDAHeader::to_buf(used, data_crc, time);

        let region_size = self.region_size.bytes();
        if Bytes((MDA_REGION_HDR_SIZE + used) as u64) > region_size {
            return Err(EngineError::Engine(ErrorEnum::Invalid,
                                           "data larger than region_size".into()));
        }

        let mut save_region = |region: usize| -> EngineResult<()> {
            let offset = BDA_STATIC_HDR_SIZE as u64 + *(region_size * region);

            try!(f.seek(SeekFrom::Start(offset)));
            try!(f.write_all(&hdr_buf));
            try!(f.write_all(data));
            try!(f.flush());

            Ok(())
        };

        let older_region = self.older();

        if let Some(updated) = self.mdas[older_region].last_updated {
            if updated >= *time {
                return Err(EngineError::Engine(ErrorEnum::Invalid,
                                               "Overwriting newer data".into()));
            }
        }

        // Save to primary and backup regions
        // TODO: Should we ignore errors?
        try!(save_region(older_region));
        try!(save_region(older_region + 2));

        self.mdas[older_region].last_updated = Some(*time);
        self.mdas[older_region].used = Bytes(used as u64);
        self.mdas[older_region].data_crc = data_crc;

        Ok(())
    }

    pub fn load_state(&self, f: &mut File) -> EngineResult<Option<Vec<u8>>> {
        let newer_region = self.newer();
        let mda = &self.mdas[newer_region];

        let mut load_region = |region: usize| {
            let offset = BDA_STATIC_HDR_SIZE as u64 + *(self.region_size * region).bytes() +
                         MDA_REGION_HDR_SIZE as u64;
            try!(f.seek(SeekFrom::Start(offset)));
            mda.load_region(f)
        };

        Ok(load_region(newer_region)
            .unwrap_or_else(|_| load_region(newer_region + 2).unwrap_or_else(|_| None)))
    }

    pub fn older(&self) -> usize {
        match self.mdas[0].last_updated.cmp(&self.mdas[1].last_updated) {
            Ordering::Less => 0,
            Ordering::Greater => 1,
            Ordering::Equal => 1,
        }
    }

    pub fn newer(&self) -> usize {
        match self.older() {
            0 => 1,
            1 => 0,
            _ => panic!("invalid val from older()"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MDAHeader {
    pub last_updated: Option<Timespec>,
    pub used: Bytes,
    pub region_size: Bytes,
    pub data_crc: u32,
}

impl MDAHeader {
    pub fn new(region_size: Bytes) -> MDAHeader {
        MDAHeader {
            last_updated: None,
            used: Bytes(0),
            region_size: region_size,
            data_crc: 0,
        }
    }

    pub fn from_buf(buf: &[u8; MDA_REGION_HDR_SIZE],
                    region_size: Bytes)
                    -> EngineResult<MDAHeader> {
        if LittleEndian::read_u32(&buf[..4]) != crc32::checksum_ieee(&buf[4..]) {
            return Err(EngineError::Engine(ErrorEnum::Invalid, "MDA region header CRC".into()));
        }

        let time = {
            match LittleEndian::read_u64(&buf[16..24]) {
                0 => None,
                // Signed cast is safe, highest order bit of each value read is guaranteed to be 0.
                secs => {
                    Some(Timespec::new(secs as i64, LittleEndian::read_u32(&buf[24..28]) as i32))
                }
            }
        };

        Ok(MDAHeader {
            used: Bytes(LittleEndian::read_u64(&buf[8..16])),
            last_updated: time,
            region_size: region_size,
            data_crc: LittleEndian::read_u32(&buf[4..8]),
        })
    }

    pub fn to_buf(data_len: usize,
                  data_crc: u32,
                  timestamp: &Timespec)
                  -> [u8; MDA_REGION_HDR_SIZE] {

        // Unsigned casts are always safe, as sec and nsec values are never negative
        assert!(timestamp.sec >= 0 && timestamp.nsec >= 0);

        let mut buf = [0u8; MDA_REGION_HDR_SIZE];

        LittleEndian::write_u32(&mut buf[4..8], data_crc);
        LittleEndian::write_u64(&mut buf[8..16], data_len as u64);
        LittleEndian::write_u64(&mut buf[16..24], timestamp.sec as u64);
        LittleEndian::write_u32(&mut buf[24..28], timestamp.nsec as u32);

        let buf_crc = crc32::checksum_ieee(&buf[4..MDA_REGION_HDR_SIZE]);
        LittleEndian::write_u32(&mut buf[..4], buf_crc);

        buf
    }

    /// Given a pre-seek()ed File, load the MDA region and return the contents
    // MDAHeader cannot seek because it doesn't know which region it's in
    pub fn load_region(&self, f: &mut File) -> EngineResult<Option<Vec<u8>>> {
        if self.used > self.region_size {
            return Err(EngineError::Engine(ErrorEnum::Invalid,
                                           format!("region mda.used {} > region_size {}",
                                                   self.used,
                                                   self.region_size)
                                               .into()));
        }

        if self.used == Bytes(0) {
            Ok(None)
        } else {
            // This cast could fail if running on a 32-bit machine and
            // size of metadata is greater than 2^32 - 1 bytes, which is
            // unlikely.
            assert!(*self.used <= std::usize::MAX as u64);
            let mut data_buf = vec![0u8; *self.used as usize];
            try!(f.read_exact(&mut data_buf));

            if self.data_crc != crc32::checksum_ieee(&data_buf) {
                return Err(EngineError::Engine(ErrorEnum::Invalid, "MDA region data CRC".into()));
            }
            Ok(Some(data_buf))
        }
    }
}

/// Validate MDA size
pub fn validate_mda_size(size: Sectors) -> EngineResult<()> {
    if size % NUM_MDA_REGIONS != Sectors(0) {
        return Err(EngineError::Engine(ErrorEnum::Invalid,
                                       format!("MDA size {} is not divisible by number of \
                                                copies required {}",
                                               size,
                                               NUM_MDA_REGIONS)));
    };

    if size < MIN_MDA_SECTORS {
        return Err(EngineError::Engine(ErrorEnum::Invalid,
                                       format!("MDA size {} is less than minimum ({})",
                                               size,
                                               MIN_MDA_SECTORS)));
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use crc::crc32;
    use time::Timespec;
    use uuid::Uuid;

    use quickcheck::{QuickCheck, TestResult};

    use types::{Bytes, Sectors};

    use super::*;

    #[test]
    /// Construct an arbitrary StaticHeader object.
    /// Write it to a buffer, read it out and make sure you get the same thing.
    fn prop_static_header() {
        fn static_header(blkdev_size: u64, mda_size_factor: u32) -> TestResult {
            let pool_uuid = Uuid::new_v4();
            let dev_uuid = Uuid::new_v4();

            let mda_size = MIN_MDA_SECTORS + Sectors((mda_size_factor * 4) as u64);
            let blkdev_size = Sectors(blkdev_size);
            let sh1 = StaticHeader::new(&pool_uuid, &dev_uuid, mda_size, blkdev_size);
            let buf = sh1.sigblock_to_buf();
            let sh2 = StaticHeader::sigblock_from_buf(&buf).unwrap();
            TestResult::from_bool(sh1.pool_uuid == sh2.pool_uuid && sh1.dev_uuid == sh2.dev_uuid &&
                                  sh1.blkdev_size == sh2.blkdev_size &&
                                  sh1.mda_size == sh2.mda_size &&
                                  sh1.reserved_size == sh2.reserved_size &&
                                  sh1.flags == sh2.flags)
        }

        QuickCheck::new()
            .tests(30)
            .quickcheck(static_header as fn(u64, u32) -> TestResult);
    }

    #[test]
    /// Using an arbitrary data buffer, construct an mda header buffer
    /// Read the mda header buffer twice.
    /// Verify that the resulting MDAHeaders have all equal components.
    /// Verify timestamp and data CRC against original values.
    fn prop_mda_header() {
        fn mda_header(data: Vec<u8>, sec: i64, nsec: i32, region_size_ext: u32) -> TestResult {
            // unwritable timestamp
            if sec < 0 || nsec < 0 {
                return TestResult::discard();
            }

            // sec value of 0 is interpreted as no timestamp when read
            if sec == 0 {
                return TestResult::discard();
            }

            // 4 is NUM_MDA_REGIONS which is not imported from super.
            let region_size = (MIN_MDA_SECTORS / 4usize).bytes() + Bytes(region_size_ext as u64);
            let timestamp = Timespec::new(sec, nsec);
            let data_crc = crc32::checksum_ieee(&data);
            let buf = MDAHeader::to_buf(data.len(), data_crc, &timestamp);
            let mda1 = MDAHeader::from_buf(&buf, region_size).unwrap();
            let mda2 = MDAHeader::from_buf(&buf, region_size).unwrap();

            TestResult::from_bool(mda1.last_updated == mda2.last_updated &&
                                  mda1.used == mda2.used &&
                                  mda1.region_size == mda2.region_size &&
                                  mda1.data_crc == mda2.data_crc &&
                                  timestamp == mda1.last_updated.unwrap() &&
                                  data_crc == mda1.data_crc)
        }

        QuickCheck::new()
            .tests(50)
            .quickcheck(mda_header as fn(Vec<u8>, i64, i32, u32) -> TestResult);
    }
}
