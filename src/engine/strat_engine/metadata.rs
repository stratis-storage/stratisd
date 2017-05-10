// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std;
use std::cmp::Ordering;
use std::str::from_utf8;
use std::io::{Read, Seek, SeekFrom, Write};

use byteorder::ByteOrder;
use byteorder::LittleEndian;
use crc::crc32;
use devicemapper::{Bytes, Sectors};
use devicemapper::consts::SECTOR_SIZE;
use time::Timespec;
use uuid::Uuid;

use super::engine::DevOwnership;
use consts::IEC;
use engine::{DevUuid, EngineResult, EngineError, ErrorEnum, PoolUuid};

const _BDA_STATIC_HDR_SECTORS: usize = 16;
pub const BDA_STATIC_HDR_SECTORS: Sectors = Sectors(_BDA_STATIC_HDR_SECTORS as u64);
const _BDA_STATIC_HDR_SIZE: usize = _BDA_STATIC_HDR_SECTORS * SECTOR_SIZE;
const BDA_STATIC_HDR_SIZE: Bytes = Bytes(_BDA_STATIC_HDR_SIZE as u64);

const MDA_RESERVED_SECTORS: Sectors = Sectors(3 * IEC::Mi / (SECTOR_SIZE as u64)); // = 3 MiB
const NUM_MDA_REGIONS: usize = 4;
const PER_MDA_REGION_COPIES: usize = 2;
const NUM_PRIMARY_MDA_REGIONS: usize = NUM_MDA_REGIONS / PER_MDA_REGION_COPIES;

const _MDA_REGION_HDR_SIZE: usize = 32;
const MDA_REGION_HDR_SIZE: Bytes = Bytes(_MDA_REGION_HDR_SIZE as u64);
pub const MIN_MDA_SECTORS: Sectors = Sectors(2032);

const STRAT_MAGIC: &'static [u8] = b"!Stra0tis\x86\xff\x02^\x41rh";

#[derive(Debug)]
pub struct BDA {
    header: StaticHeader,
    regions: MDARegions,
}

impl BDA {
    /// Initialize a blockdev with a Stratis BDA.
    pub fn initialize<F>(mut f: &mut F,
                         pool_uuid: &Uuid,
                         dev_uuid: &Uuid,
                         mda_size: Sectors,
                         blkdev_size: Sectors)
                         -> EngineResult<BDA>
        where F: Seek + Write
    {
        let zeroed = [0u8; _BDA_STATIC_HDR_SIZE];
        let header = StaticHeader::new(pool_uuid, dev_uuid, mda_size, blkdev_size);
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

    pub fn load<F>(f: &mut F) -> EngineResult<Option<BDA>>
        where F: Read + Seek
    {
        let header = try!(StaticHeader::setup(f));
        if header.is_none() {
            return Ok(None);
        }
        let header = header.expect("must have exited if None");

        let regions = try!(MDARegions::load(&header, f));

        Ok(Some(BDA {
                    header: header,
                    regions: regions,
                }))
    }

    /// Zero out Static Header on the blockdev. This causes it to no
    /// longer be seen as a Stratis blockdev.
    pub fn wipe<F>(f: &mut F) -> EngineResult<()>
        where F: Seek + Write
    {
        let zeroed = [0u8; _BDA_STATIC_HDR_SIZE];

        // Wiping Static Header should do it
        try!(f.seek(SeekFrom::Start(0)));
        try!(f.write_all(&zeroed));
        try!(f.flush());
        Ok(())
    }

    /// Save metadata to the disk
    pub fn save_state<F>(&mut self,
                         time: &Timespec,
                         metadata: &[u8],
                         mut f: &mut F)
                         -> EngineResult<()>
        where F: Seek + Write
    {
        self.regions.save_state(time, metadata, &mut f)
    }

    /// Read latest metadata from the disk
    pub fn load_state<F>(&self, mut f: &mut F) -> EngineResult<Option<Vec<u8>>>
        where F: Read + Seek
    {
        self.regions.load_state(&mut f)
    }

    /// The time when the most recent metadata was written to the BDA,
    /// if any.
    pub fn last_update_time(&self) -> Option<&Timespec> {
        self.regions.last_update_time()
    }

    /// The UUID of the device.
    pub fn dev_uuid(&self) -> &DevUuid {
        &self.header.dev_uuid
    }

    /// The UUID of the device's pool.
    pub fn pool_uuid(&self) -> &PoolUuid {
        &self.header.pool_uuid
    }

    /// The size of the device.
    pub fn dev_size(&self) -> Sectors {
        self.header.blkdev_size
    }

    /// The number of sectors the BDA itself occupies.
    pub fn size(&self) -> Sectors {
        BDA_STATIC_HDR_SECTORS + self.header.mda_size + self.header.reserved_size
    }
}

#[derive(Debug)]
pub struct StaticHeader {
    blkdev_size: Sectors,
    pool_uuid: PoolUuid,
    dev_uuid: DevUuid,
    mda_size: Sectors,
    reserved_size: Sectors,
    flags: u64,
}

impl StaticHeader {
    pub fn new(pool_uuid: &PoolUuid,
               dev_uuid: &DevUuid,
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
    /// If there is no StaticHeader on the device, return None.
    /// If there is a problem reading a header, return an error.
    fn setup<F>(f: &mut F) -> EngineResult<Option<StaticHeader>>
        where F: Read + Seek
    {
        try!(f.seek(SeekFrom::Start(0)));
        let mut buf = [0u8; _BDA_STATIC_HDR_SIZE];
        try!(f.read(&mut buf));

        // TODO: repair static header if one incorrect?
        // Note: this would require some adjustment or some revision to
        // setup_from_buf().
        StaticHeader::setup_from_buf(&buf)
    }

    /// Try to find a valid StaticHeader in a buffer.
    /// If there is an error in reading the first, try the next. If there is
    /// no error in reading the first, assume it is correct, i.e., do not
    /// verify that it matches the next.
    /// Return None if the static header's magic number is wrong.
    fn setup_from_buf(buf: &[u8; _BDA_STATIC_HDR_SIZE]) -> EngineResult<Option<StaticHeader>> {
        let sigblock_spots = [&buf[SECTOR_SIZE..2 * SECTOR_SIZE],
                              &buf[9 * SECTOR_SIZE..10 * SECTOR_SIZE]];

        for buf in &sigblock_spots {
            match StaticHeader::sigblock_from_buf(buf) {
                Ok(val) => return Ok(val),
                _ => continue,
            }
        }

        Err(EngineError::Engine(ErrorEnum::Invalid, "not a Stratis device".into()))
    }

    /// Determine the ownership of a device.
    pub fn determine_ownership<F>(f: &mut F) -> EngineResult<DevOwnership>
        where F: Read + Seek
    {


        try!(f.seek(SeekFrom::Start(0)));
        let mut buf = [0u8; _BDA_STATIC_HDR_SIZE];
        try!(f.read(&mut buf));

        // Using setup() as a test of ownership sets a high bar. It is
        // not sufficient to have STRAT_MAGIC to be considered "Ours",
        // it must also have correct CRC, no weird stuff in fields,
        // etc!
        match StaticHeader::setup_from_buf(&buf) {
            Ok(Some(sh)) => Ok(DevOwnership::Ours(sh.pool_uuid)),
            Ok(None) => {
                if buf.iter().any(|x| *x != 0) {
                    Ok(DevOwnership::Theirs)
                } else {
                    Ok(DevOwnership::Unowned)
                }
            }
            Err(err) => Err(err),
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
    fn sigblock_from_buf(buf: &[u8]) -> EngineResult<Option<StaticHeader>> {

        assert_eq!(buf.len(), SECTOR_SIZE);

        if &buf[4..20] != STRAT_MAGIC {
            return Ok(None);
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

        Ok(Some(StaticHeader {
                    pool_uuid: pool_uuid,
                    dev_uuid: dev_uuid,
                    blkdev_size: blkdev_size,
                    mda_size: mda_size,
                    reserved_size: Sectors(LittleEndian::read_u64(&buf[104..112])),
                    flags: 0,
                }))
    }
}

#[derive(Debug)]
pub struct MDARegions {
    // Spec defines 4 regions, but regions 2 & 3 are duplicates of 0 and 1 respectively
    region_size: Sectors,
    mdas: [MDAHeader; NUM_PRIMARY_MDA_REGIONS],
}

impl MDARegions {
    /// Calculate the offset from start of device for an MDARegion.
    fn mda_offset(index: usize, per_region_size: Bytes) -> u64 {
        *(BDA_STATIC_HDR_SIZE + per_region_size * index)
    }

    pub fn initialize<F>(header: &StaticHeader, f: &mut F) -> EngineResult<MDARegions>
        where F: Seek + Write
    {
        let hdr_buf = [0u8; _MDA_REGION_HDR_SIZE];

        let region_size = header.mda_size / NUM_MDA_REGIONS;
        let per_region_size = region_size.bytes();
        for region in 0..NUM_MDA_REGIONS {
            try!(f.seek(SeekFrom::Start(MDARegions::mda_offset(region, per_region_size))));
            try!(f.write_all(&hdr_buf));
        }

        try!(f.flush());

        Ok(MDARegions {
               region_size: region_size,
               mdas: [MDAHeader::new(per_region_size),
                      MDAHeader::new(per_region_size)],
           })
    }

    // Construct MDARegions based on on-disk info
    pub fn load<F>(header: &StaticHeader, f: &mut F) -> EngineResult<MDARegions>
        where F: Read + Seek
    {
        let region_size = header.mda_size / NUM_MDA_REGIONS;
        let per_region_size = region_size.bytes();

        let mut load_a_region = |region: usize| -> EngineResult<MDAHeader> {
            let mut hdr_buf = [0u8; _MDA_REGION_HDR_SIZE];
            try!(f.seek(SeekFrom::Start(MDARegions::mda_offset(region, per_region_size))));
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
    pub fn save_state<F>(&mut self, time: &Timespec, data: &[u8], f: &mut F) -> EngineResult<()>
        where F: Seek + Write
    {
        let used = data.len();
        let data_crc = crc32::checksum_ieee(data);
        let hdr_buf = MDAHeader::to_buf(used, data_crc, time);

        let region_size = self.region_size.bytes();
        try!(check_mda_region_size(Bytes(used as u64), region_size));

        let mut save_region = |region: usize| -> EngineResult<()> {
            try!(f.seek(SeekFrom::Start(MDARegions::mda_offset(region, region_size))));
            try!(f.write_all(&hdr_buf));
            try!(f.write_all(data));
            try!(f.flush());

            Ok(())
        };

        if self.last_update_time() >= Some(time) {
            return Err(EngineError::Engine(ErrorEnum::Invalid, "Overwriting newer data".into()));
        }

        let older_region = self.older();

        // Save to primary and backup regions
        // TODO: Should we ignore errors?
        try!(save_region(older_region));
        try!(save_region(older_region + 2));

        self.mdas[older_region].last_updated = Some(*time);
        self.mdas[older_region].used = Some(Bytes(used as u64));
        self.mdas[older_region].data_crc = Some(data_crc);

        Ok(())
    }

    pub fn load_state<F>(&self, f: &mut F) -> EngineResult<Option<Vec<u8>>>
        where F: Read + Seek
    {
        let newer_region = self.newer();
        let mda = &self.mdas[newer_region];

        let mut load_region = |region: usize| {
            let offset = MDARegions::mda_offset(region, self.region_size.bytes()) +
                         _MDA_REGION_HDR_SIZE as u64;
            try!(f.seek(SeekFrom::Start(offset)));
            mda.load_region(f)
        };

        Ok(load_region(newer_region).unwrap_or_else(|_| {
                                                        load_region(newer_region + 2)
                                                            .unwrap_or_else(|_| None)
                                                    }))
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

    /// The last update time for these MDA regions
    pub fn last_update_time(&self) -> Option<&Timespec> {
        self.mdas[self.newer()].last_updated.as_ref()
    }
}

#[derive(Debug)]
pub struct MDAHeader {
    last_updated: Option<Timespec>,

    /// Size of region used for pool metadata.
    used: Option<Bytes>,

    /// Total size of region, including both the header and space used for
    /// pool metadata.
    region_size: Bytes,

    data_crc: Option<u32>,
}

impl MDAHeader {
    pub fn new(region_size: Bytes) -> MDAHeader {
        MDAHeader {
            last_updated: None,
            used: None,
            region_size: region_size,
            data_crc: None,
        }
    }

    /// Get an MDAHeader from the buffer.
    /// Return an error for a bad checksum.
    /// Return an error if the size of the region used is too large for the given region_size.
    pub fn from_buf(buf: &[u8; _MDA_REGION_HDR_SIZE],
                    region_size: Bytes)
                    -> EngineResult<MDAHeader> {
        if LittleEndian::read_u32(&buf[..4]) != crc32::checksum_ieee(&buf[4..]) {
            return Err(EngineError::Engine(ErrorEnum::Invalid, "MDA region header CRC".into()));
        }

        match LittleEndian::read_u64(&buf[16..24]) {
            0 => {
                Ok(MDAHeader {
                       used: None,
                       last_updated: None,
                       data_crc: None,
                       region_size: region_size,
                   })
            }
            secs => {
                let used = Bytes(LittleEndian::read_u64(&buf[8..16]));
                try!(check_mda_region_size(used, region_size));

                let nsecs = LittleEndian::read_u32(&buf[24..28]);
                // Signed cast is safe, highest order bit of each value
                // read is guaranteed to be 0.
                assert!(nsecs <= std::i32::MAX as u32);
                assert!(secs <= std::i64::MAX as u64);

                Ok(MDAHeader {
                       used: Some(used),
                       last_updated: Some(Timespec::new(secs as i64, nsecs as i32)),
                       data_crc: Some(LittleEndian::read_u32(&buf[4..8])),
                       region_size: region_size,
                   })
            }
        }
    }

    pub fn to_buf(data_len: usize,
                  data_crc: u32,
                  timestamp: &Timespec)
                  -> [u8; _MDA_REGION_HDR_SIZE] {
        // Unsigned casts are always safe, as sec and nsec values are never negative
        assert!(timestamp.sec >= 0 && timestamp.nsec >= 0);

        let mut buf = [0u8; _MDA_REGION_HDR_SIZE];

        LittleEndian::write_u32(&mut buf[4..8], data_crc);
        LittleEndian::write_u64(&mut buf[8..16], data_len as u64);
        LittleEndian::write_u64(&mut buf[16..24], timestamp.sec as u64);
        LittleEndian::write_u32(&mut buf[24..28], timestamp.nsec as u32);

        let buf_crc = crc32::checksum_ieee(&buf[4.._MDA_REGION_HDR_SIZE]);
        LittleEndian::write_u32(&mut buf[..4], buf_crc);

        buf
    }

    /// Given a pre-seek()ed File, load the MDA region and return the contents
    // MDAHeader cannot seek because it doesn't know which region it's in
    pub fn load_region<F>(&self, f: &mut F) -> EngineResult<Option<Vec<u8>>>
        where F: Read
    {
        if let Some(used) = self.used {
            // This should never fail, since the property is checked when the MDAHeader is loaded
            assert!(MDA_REGION_HDR_SIZE + used <= self.region_size);
            // This cast could fail if running on a 32-bit machine and
            // size of metadata is greater than 2^32 - 1 bytes, which is
            // unlikely.
            assert!(*used <= std::usize::MAX as u64);
            let mut data_buf = vec![0u8; *used as usize];
            try!(f.read_exact(&mut data_buf));

            if self.data_crc
                   .expect("Option constructors of 'data_crc' and 'used' are always the same.") !=
               crc32::checksum_ieee(&data_buf) {
                return Err(EngineError::Engine(ErrorEnum::Invalid, "MDA region data CRC".into()));
            }
            Ok(Some(data_buf))
        } else {
            Ok(None)
        }
    }
}


/// Check that data size does not exceed region available.
/// Note that used is the amount used for metadata only.
fn check_mda_region_size(used: Bytes, available: Bytes) -> EngineResult<()> {
    if MDA_REGION_HDR_SIZE + used > available {
        return Err(EngineError::Engine(ErrorEnum::Invalid,
                                       format!("metadata length {} exceeds region available {}",
                                               used,
                                               // available region > header size
                                               available - MDA_REGION_HDR_SIZE)
                                               .into()));
    };
    Ok(())
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
    use std::io::Cursor;

    use crc::crc32;
    use devicemapper::{Bytes, Sectors};
    use quickcheck::{QuickCheck, TestResult};
    use time::{now, Timespec};
    use uuid::Uuid;

    use consts::IEC;

    use super::super::engine::DevOwnership;
    use super::*;

    /// Return a static header with random block device and MDA size.
    /// The block device is less than the minimum, for efficiency in testing.
    fn random_static_header(blkdev_size: u64, mda_size_factor: u32) -> StaticHeader {
        let pool_uuid = Uuid::new_v4();
        let dev_uuid = Uuid::new_v4();
        let mda_size = MIN_MDA_SECTORS + Sectors((mda_size_factor * 4) as u64);
        let blkdev_size = (Bytes(IEC::Mi) + Sectors(blkdev_size).bytes()).sectors();
        StaticHeader::new(&pool_uuid, &dev_uuid, mda_size, blkdev_size)
    }

    #[test]
    /// Verify that the file is theirs, if there are any non-zero bits in BDA.
    /// Unowned if all bits are 0.
    fn test_other_ownership() {
        fn property(offset: u8, length: u8, value: u8) -> TestResult {
            if value == 0 || length == 0 {
                return TestResult::discard();
            }
            let mut buf = Cursor::new(vec![0; _BDA_STATIC_HDR_SIZE]);
            match StaticHeader::determine_ownership(&mut buf).unwrap() {
                DevOwnership::Unowned => {}
                _ => return TestResult::failed(),
            }

            let data = vec![value; length as usize];
            buf.seek(SeekFrom::Start(offset as u64)).unwrap();
            buf.write(&data).unwrap();
            match StaticHeader::determine_ownership(&mut buf).unwrap() {
                DevOwnership::Theirs => {}
                _ => return TestResult::failed(),
            }
            TestResult::passed()

        }
        QuickCheck::new()
            .tests(10)
            .quickcheck(property as fn(u8, u8, u8) -> TestResult);
    }

    #[test]
    /// Construct an arbitrary StaticHeader object.
    /// Verify that the "file" is unowned.
    /// Initialize a BDA.
    /// Verify that Stratis owns the file.
    /// Wipe the BDA.
    /// Verify that the file is again unowned.
    fn prop_test_ownership() {
        fn test_ownership(blkdev_size: u64, mda_size_factor: u32) -> TestResult {
            let sh = random_static_header(blkdev_size, mda_size_factor);
            let pool_uuid = sh.pool_uuid;
            let mut buf = Cursor::new(vec![0; *sh.blkdev_size.bytes() as usize]);
            let ownership = StaticHeader::determine_ownership(&mut buf).unwrap();
            match ownership {
                DevOwnership::Unowned => {}
                _ => return TestResult::failed(),
            }

            BDA::initialize(&mut buf,
                            &sh.pool_uuid,
                            &sh.dev_uuid,
                            sh.mda_size,
                            sh.blkdev_size)
                    .unwrap();
            let ownership = StaticHeader::determine_ownership(&mut buf).unwrap();
            match ownership {
                DevOwnership::Ours(uuid) => {
                    if pool_uuid != uuid {
                        return TestResult::failed();
                    }
                }
                _ => return TestResult::failed(),
            }

            BDA::wipe(&mut buf).unwrap();
            let ownership = StaticHeader::determine_ownership(&mut buf).unwrap();
            match ownership {
                DevOwnership::Unowned => {}
                _ => return TestResult::failed(),
            }

            TestResult::passed()
        }
        QuickCheck::new()
            .tests(20)
            .quickcheck(test_ownership as fn(u64, u32) -> TestResult);
    }

    #[test]
    /// Construct an arbitrary StaticHeader object.
    /// Initialize a BDA.
    /// Verify that the last update time is None.
    fn prop_empty_bda() {
        fn empty_bda(blkdev_size: u64, mda_size_factor: u32) -> TestResult {
            let sh = random_static_header(blkdev_size, mda_size_factor);
            let mut buf = Cursor::new(vec![0; *sh.blkdev_size.bytes() as usize]);
            let bda = BDA::initialize(&mut buf,
                                      &sh.pool_uuid,
                                      &sh.dev_uuid,
                                      sh.mda_size,
                                      sh.blkdev_size)
                    .unwrap();
            TestResult::from_bool(bda.last_update_time().is_none())
        }

        QuickCheck::new()
            .tests(20)
            .quickcheck(empty_bda as fn(u64, u32) -> TestResult);
    }

    #[test]
    /// Construct a BDA and verify that an error is returned if timestamp
    /// of saved data is older than timestamp of most recently written data.
    fn test_early_times_err() {
        let data = [0u8; 3];

        // Construct a BDA.
        let sh = random_static_header(0, 0);
        let mut buf = Cursor::new(vec![0; *sh.blkdev_size.bytes() as usize]);
        let mut bda = BDA::initialize(&mut buf,
                                      &sh.pool_uuid,
                                      &sh.dev_uuid,
                                      sh.mda_size,
                                      sh.blkdev_size)
                .unwrap();

        let timestamp0 = now().to_timespec();
        let timestamp1 = now().to_timespec();
        assert!(timestamp0 != timestamp1);

        let mut buf = Cursor::new(vec![0; *sh.blkdev_size.bytes() as usize]);
        bda.save_state(&timestamp1, &data, &mut buf).unwrap();

        // Error, because current timestamp is older than written to newer.
        assert!(bda.save_state(&timestamp0, &data, &mut buf).is_err());

        let timestamp2 = now().to_timespec();
        let timestamp3 = now().to_timespec();
        assert!(timestamp2 != timestamp3);

        bda.save_state(&timestamp3, &data, &mut buf).unwrap();

        // Error, because current timestamp is older than written to newer.
        assert!(bda.save_state(&timestamp2, &data, &mut buf).is_err());
    }


    #[test]
    /// Construct an arbitrary StaticHeader object.
    /// Initialize a BDA.
    /// Save metadata and verify correct update time and state.
    /// Reload BDA and verify that new BDA has correct update time.
    /// Load state using new BDA and verify correct state.
    /// Save metadata again, and reload one more time, verifying new timestamp.
    fn prop_check_state() {
        fn check_state(blkdev_size: u64,
                       mda_size_factor: u32,
                       state: Vec<u8>,
                       next_state: Vec<u8>)
                       -> TestResult {
            let sh = random_static_header(blkdev_size, mda_size_factor);
            let mut buf = Cursor::new(vec![0; *sh.blkdev_size.bytes() as usize]);
            let mut bda = BDA::initialize(&mut buf,
                                          &sh.pool_uuid,
                                          &sh.dev_uuid,
                                          sh.mda_size,
                                          sh.blkdev_size)
                    .unwrap();
            let current_time = now().to_timespec();
            bda.save_state(&current_time, &state, &mut buf).unwrap();
            let loaded_state = bda.load_state(&mut buf).unwrap();

            if let Some(t) = bda.last_update_time() {
                if t != &current_time {
                    return TestResult::failed();
                }
            } else {
                return TestResult::failed();
            }

            if let Some(s) = loaded_state {
                if s != state {
                    return TestResult::failed();
                }
            } else {
                return TestResult::failed();
            }

            let mut bda = BDA::load(&mut buf).unwrap().unwrap();
            let loaded_state = bda.load_state(&mut buf).unwrap();

            if let Some(s) = loaded_state {
                if s != state {
                    return TestResult::failed();
                }
            } else {
                return TestResult::failed();
            }

            if let Some(t) = bda.last_update_time() {
                if t != &current_time {
                    return TestResult::failed();
                }
            } else {
                return TestResult::failed();
            }

            let current_time = now().to_timespec();
            bda.save_state(&current_time, &next_state, &mut buf)
                .unwrap();
            let loaded_state = bda.load_state(&mut buf).unwrap();

            if let Some(s) = loaded_state {
                if s != next_state {
                    return TestResult::failed();
                }
            } else {
                return TestResult::failed();
            }

            if let Some(t) = bda.last_update_time() {
                if t != &current_time {
                    return TestResult::failed();
                }
            } else {
                return TestResult::failed();
            }

            TestResult::passed()
        }

        QuickCheck::new()
            .tests(20)
            .quickcheck(check_state as fn(u64, u32, Vec<u8>, Vec<u8>) -> TestResult);
    }

    #[test]
    /// Construct an arbitrary StaticHeader object.
    /// Write it to a buffer, read it out and make sure you get the same thing.
    fn prop_static_header() {
        fn static_header(blkdev_size: u64, mda_size_factor: u32) -> TestResult {
            let sh1 = random_static_header(blkdev_size, mda_size_factor);
            let buf = sh1.sigblock_to_buf();
            let sh2 = StaticHeader::sigblock_from_buf(&buf).unwrap().unwrap();
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
                                  data_crc == mda1.data_crc.unwrap())
        }

        QuickCheck::new()
            .tests(50)
            .quickcheck(mda_header as fn(Vec<u8>, i64, i32, u32) -> TestResult);
    }

    /// Verify that bad crc causes an error.
    #[test]
    fn test_from_buf_crc_error() {
        let data = [0u8; 3];
        let timestamp = now().to_timespec();
        let data_crc = crc32::checksum_ieee(&data);
        let mut buf = MDAHeader::to_buf(data.len(), data_crc, &timestamp);
        LittleEndian::write_u32(&mut buf[..4], 0u32);
        assert!(MDAHeader::from_buf(&buf, Bytes(data.len() as u64) + MDA_REGION_HDR_SIZE).is_err());
    }

    /// Verify that too small region_size causes an error.
    #[test]
    fn test_from_buf_size_error() {
        let data = [0u8; 3];
        let timestamp = now().to_timespec();
        let data_crc = crc32::checksum_ieee(&data);
        let buf = MDAHeader::to_buf(data.len(), data_crc, &timestamp);
        assert!(MDAHeader::from_buf(&buf, MDA_REGION_HDR_SIZE).is_err());
    }
}
