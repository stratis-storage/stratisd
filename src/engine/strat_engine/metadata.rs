// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cmp::Ordering;
use std::str::from_utf8;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use byteorder::ByteOrder;
use byteorder::LittleEndian;
use crc::crc32;
use time::Timespec;
use uuid::Uuid;

use types::Sectors;
use super::engine::DevOwnership;
use consts::{SECTOR_SIZE, MEGA};
use engine::{EngineResult, EngineError, ErrorEnum};

pub const BDA_STATIC_HDR_SECTORS: u64 = 16;
const BDA_STATIC_HDR_SIZE: u64 = BDA_STATIC_HDR_SECTORS * SECTOR_SIZE;
const MDA_RESERVED_SIZE: Sectors = Sectors(3 * MEGA / SECTOR_SIZE); // = 3 MiB
const NUM_MDA_REGIONS: u64 = 4;
const PER_MDA_REGION_COPIES: u64 = 2;
const NUM_PRIMARY_MDA_REGIONS: u64 = NUM_MDA_REGIONS / PER_MDA_REGION_COPIES;
const MDA_REGION_HDR_SIZE: usize = 32;
pub const MIN_MDA_SIZE: Sectors = Sectors(2040);
const STRAT_MAGIC: &'static [u8] = b"!Stra0tis\x86\xff\x02^\x41rh";

#[derive(Debug)]
pub struct BDA {
    pub header: StaticHeader,
    pub regions: MDARegions,
}

impl BDA {
    /// Initialize a blockdev with a Stratis BDA.
    pub fn initialize(mut f: &mut File, header: StaticHeader) -> EngineResult<BDA> {
        let zeroed = [0u8; BDA_STATIC_HDR_SIZE as usize];
        let hdr_buf = header.sigblock_to_buf();

        // Write 8K header. Static_Header copies go in sectors 1 and 9.
        try!(f.seek(SeekFrom::Start(0)));
        try!(f.write_all(&zeroed[..SECTOR_SIZE as usize]));
        try!(f.write_all(&hdr_buf));
        try!(f.write_all(&zeroed[(SECTOR_SIZE * 7) as usize..]));
        try!(f.flush());
        try!(f.write_all(&hdr_buf));
        try!(f.write_all(&zeroed[(SECTOR_SIZE * 6) as usize..]));
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
        let zeroed = [0u8; BDA_STATIC_HDR_SIZE as usize];

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
            reserved_size: MDA_RESERVED_SIZE,
            flags: 0,
        }
    }

    /// Try to find a valid StaticHeader on a device.
    pub fn setup(f: &mut File) -> EngineResult<StaticHeader> {
        try!(f.seek(SeekFrom::Start(0)));
        let mut buf = [0u8; BDA_STATIC_HDR_SIZE as usize];
        try!(f.read(&mut buf));

        Self::setup_from_buf(&buf)
    }

    /// Try to find a valid StaticHeader in a buffer.
    pub fn setup_from_buf(buf: &[u8; BDA_STATIC_HDR_SIZE as usize]) -> EngineResult<StaticHeader> {
        let sigblock_spots = [&buf[SECTOR_SIZE as usize..2 * SECTOR_SIZE as usize],
                              &buf[9 * SECTOR_SIZE as usize..10 * SECTOR_SIZE as usize]];

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
        let mut buf = [0u8; BDA_STATIC_HDR_SIZE as usize];
        try!(f.read(&mut buf));

        // Using setup() as a test of ownership sets a high bar. It is
        // not sufficient to have STRAT_MAGIC to be considered "Ours",
        // it must also have correct CRC, no weird stuff in fields,
        // etc!
        match Self::setup_from_buf(&buf) {
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
    pub fn sigblock_to_buf(&self) -> [u8; SECTOR_SIZE as usize] {
        let mut buf = [0u8; SECTOR_SIZE as usize];
        buf[4..20].clone_from_slice(STRAT_MAGIC);
        LittleEndian::write_u64(&mut buf[20..28], *self.blkdev_size);
        buf[32..64].clone_from_slice(self.pool_uuid.simple().to_string().as_bytes());
        buf[64..96].clone_from_slice(self.dev_uuid.simple().to_string().as_bytes());
        LittleEndian::write_u32(&mut buf[160..164], *self.mda_size as u32);
        LittleEndian::write_u32(&mut buf[164..168], *self.reserved_size as u32);

        let hdr_crc = crc32::checksum_ieee(&buf[4..SECTOR_SIZE as usize]);
        LittleEndian::write_u32(&mut buf[..4], hdr_crc);
        buf
    }

    /// Build a StaticHeader from a SECTOR_SIZE buf that was read from
    /// a blockdev.
    pub fn sigblock_from_buf(buf: &[u8]) -> EngineResult<StaticHeader> {

        assert_eq!(buf.len(), SECTOR_SIZE as usize);

        if &buf[4..20] != STRAT_MAGIC {
            return Err(EngineError::Engine(ErrorEnum::Invalid, "No Stratis magic".into()));
        }

        let crc = crc32::checksum_ieee(&buf[4..SECTOR_SIZE as usize]);
        if crc != LittleEndian::read_u32(&buf[..4]) {
            return Err(EngineError::Engine(ErrorEnum::Invalid, "header CRC invalid".into()));
        }

        let blkdev_size = Sectors(LittleEndian::read_u64(&buf[20..28]) as u64);

        let pool_uuid = try!(Uuid::parse_str(try!(from_utf8(&buf[32..64]))));
        let dev_uuid = try!(Uuid::parse_str(try!(from_utf8(&buf[64..96]))));

        let mda_size = Sectors(LittleEndian::read_u64(&buf[96..104]) as u64);

        try!(validate_mda_size(mda_size));

        Ok(StaticHeader {
            pool_uuid: pool_uuid,
            dev_uuid: dev_uuid,
            blkdev_size: blkdev_size,
            mda_size: mda_size,
            reserved_size: Sectors(LittleEndian::read_u64(&buf[104..112]) as u64),
            flags: 0,
        })
    }
}

#[derive(Debug)]
pub struct MDARegions {
    // Spec defines 4 regions, but regions 2 & 3 are duplicates of 0 and 1 respectively
    pub region_size: Sectors,
    pub mdas: [MDAHeader; NUM_PRIMARY_MDA_REGIONS as usize],
}

impl MDARegions {
    pub fn initialize(header: &StaticHeader, f: &mut File) -> EngineResult<MDARegions> {
        let region_size = Sectors(*header.mda_size / NUM_MDA_REGIONS);
        let per_region_size: u64 = *region_size * SECTOR_SIZE;
        let hdr_buf = [0u8; MDA_REGION_HDR_SIZE];

        for region in 0..NUM_MDA_REGIONS {
            try!(f.seek(SeekFrom::Start(BDA_STATIC_HDR_SIZE + (region as u64 * per_region_size))));
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
        let region_size = Sectors(*header.mda_size / NUM_MDA_REGIONS);
        let per_region_size: u64 = *region_size * SECTOR_SIZE;

        let mut load_a_region = |region: u8| -> EngineResult<MDAHeader> {
            let mut hdr_buf = [0u8; MDA_REGION_HDR_SIZE];
            let offset = BDA_STATIC_HDR_SIZE + (region as u64 * *region_size * SECTOR_SIZE);

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
        let region_size: u64 = *self.region_size * SECTOR_SIZE;

        let mut save_region = |region: usize| -> EngineResult<()> {
            let hdr_buf = MDAHeader::to_buf(data, time);
            let offset = BDA_STATIC_HDR_SIZE + (region as u64 * region_size);

            if MDA_REGION_HDR_SIZE + data.len() > region_size as usize {
                return Err(EngineError::Engine(ErrorEnum::Invalid,
                                               "data larger than region_size".into()));
            }

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
        self.mdas[older_region].used = data.len() as u64;

        Ok(())
    }

    pub fn load_state(&self, f: &mut File) -> EngineResult<Option<Vec<u8>>> {
        let newer_region = self.newer();
        let mda = &self.mdas[newer_region];

        let mut load_region = |region| {
            let offset = BDA_STATIC_HDR_SIZE + (region as u64 * *self.region_size * SECTOR_SIZE) +
                         MDA_REGION_HDR_SIZE as u64;
            try!(f.seek(SeekFrom::Start(offset)));
            mda.load_region(f)
        };

        Ok(load_region(newer_region)
            .unwrap_or_else(|_| load_region(newer_region + 2).unwrap_or_else(|_| None)))
    }

    pub fn older(&self) -> usize {
        match self.mdas[0].last_updated.cmp(&self.mdas[1].last_updated) {
            Ordering::Less => 1,
            Ordering::Greater => 0,
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
    pub used: u64,
    pub region_size: u64,
    pub data_crc: u32,
}

impl MDAHeader {
    pub fn new(region_size: u64) -> MDAHeader {
        MDAHeader {
            last_updated: None,
            used: 0,
            region_size: region_size,
            data_crc: 0,
        }
    }

    pub fn from_buf(buf: &[u8; MDA_REGION_HDR_SIZE], region_size: u64) -> EngineResult<MDAHeader> {
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
            used: LittleEndian::read_u64(&buf[8..16]),
            last_updated: time,
            region_size: region_size,
            data_crc: LittleEndian::read_u32(&buf[4..8]),
        })
    }

    pub fn to_buf(data: &[u8], timestamp: &Timespec) -> [u8; MDA_REGION_HDR_SIZE] {

        // Unsigned casts are always safe, as sec and nsec values are never negative
        assert!(timestamp.sec >= 0 && timestamp.nsec >= 0);

        let mut buf = [0u8; MDA_REGION_HDR_SIZE];

        LittleEndian::write_u32(&mut buf[4..8], crc32::checksum_ieee(data));
        LittleEndian::write_u64(&mut buf[8..16], data.len() as u64);
        LittleEndian::write_u64(&mut buf[16..24], timestamp.sec as u64);
        LittleEndian::write_u32(&mut buf[24..28], timestamp.nsec as u32);

        let buf_crc = crc32::checksum_ieee(&buf[4..MDA_REGION_HDR_SIZE]);
        LittleEndian::write_u32(&mut buf[..4], buf_crc);

        buf
    }

    /// Given a pre-seek()ed File, load the MDA region and return the contents
    // MDAHeader cannot seek because it doesn't know which region it's in
    pub fn load_region(&self, f: &mut File) -> EngineResult<Option<Vec<u8>>> {
        let mut hdr_buf = [0u8; MDA_REGION_HDR_SIZE];
        try!(f.read_exact(&mut hdr_buf));

        if self.used > self.region_size {
            return Err(EngineError::Engine(ErrorEnum::Invalid,
                                           format!("region mda.used {} > region_size {}",
                                                   self.used,
                                                   self.region_size)
                                               .into()));
        }

        if self.used == 0 {
            Ok(None)
        } else {
            let mut data_buf = vec![0u8; self.used as usize];
            try!(f.read_exact(&mut data_buf));

            if self.data_crc != crc32::checksum_ieee(&data_buf) {
                return Err(EngineError::Engine(ErrorEnum::Invalid, "MDA region data CRC".into()));
            }
            Ok(Some(data_buf))
        }
    }
}

/// Validate MDA size
/// Return None if MDA size is fine, otherwise a message.
pub fn validate_mda_size(size: Sectors) -> EngineResult<()> {
    if *size % NUM_MDA_REGIONS != 0 {
        return Err(EngineError::Engine(ErrorEnum::Invalid,
                                       format!("MDA size {} is not divisible by number of \
                                                copies required {}",
                                               *size,
                                               NUM_MDA_REGIONS)));
    };

    if size < MIN_MDA_SIZE {
        return Err(EngineError::Engine(ErrorEnum::Invalid,
                                       format!("MDA size {} is less than minimum ({})",
                                               *size,
                                               *MIN_MDA_SIZE)));
    };
    Ok(())
}
