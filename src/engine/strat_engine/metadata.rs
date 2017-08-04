// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::str::from_utf8;
use std::io::{Read, Seek, SeekFrom, Write};

use byteorder::{ByteOrder, LittleEndian};
use crc::crc32;
use time::Timespec;
use uuid::Uuid;

use devicemapper::{Bytes, Sectors};
use devicemapper::consts::SECTOR_SIZE;

use super::super::consts::IEC;
use super::super::errors::{EngineResult, EngineError, ErrorEnum};
use super::super::types::{DevUuid, PoolUuid};

use super::engine::DevOwnership;

pub use self::mda::{MIN_MDA_SECTORS, validate_mda_size};

const _BDA_STATIC_HDR_SECTORS: usize = 16;
pub const BDA_STATIC_HDR_SECTORS: Sectors = Sectors(_BDA_STATIC_HDR_SECTORS as u64);
const _BDA_STATIC_HDR_SIZE: usize = _BDA_STATIC_HDR_SECTORS * SECTOR_SIZE;
const BDA_STATIC_HDR_SIZE: Bytes = Bytes(_BDA_STATIC_HDR_SIZE as u64);

const MDA_RESERVED_SECTORS: Sectors = Sectors(3 * IEC::Mi / (SECTOR_SIZE as u64)); // = 3 MiB

const STRAT_MAGIC: &'static [u8] = b"!Stra0tis\x86\xff\x02^\x41rh";

#[derive(Debug)]
pub struct BDA {
    header: StaticHeader,
    regions: mda::MDARegions,
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
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&zeroed[..SECTOR_SIZE])?;
        f.write_all(&hdr_buf)?;
        f.write_all(&zeroed[SECTOR_SIZE * 7..])?;
        f.flush()?;
        f.write_all(&hdr_buf)?;
        f.write_all(&zeroed[SECTOR_SIZE * 6..])?;
        f.flush()?;

        let regions = mda::MDARegions::initialize(BDA_STATIC_HDR_SIZE, header.mda_size, &mut f)?;

        Ok(BDA {
               header: header,
               regions: regions,
           })
    }

    /// Load a BDA on initial setup of a device.
    /// Returns None if no BDA appears to exist.
    pub fn load<F>(f: &mut F) -> EngineResult<Option<BDA>>
        where F: Read + Seek
    {
        let header = match StaticHeader::setup(f)? {
            Some(header) => header,
            None => return Ok(None),
        };

        let regions = mda::MDARegions::load(BDA_STATIC_HDR_SIZE, header.mda_size, f)?;

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
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&zeroed)?;
        f.flush()?;
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
        self.regions
            .save_state(BDA_STATIC_HDR_SIZE, time, metadata, &mut f)
    }

    /// Read latest metadata from the disk
    pub fn load_state<F>(&self, mut f: &mut F) -> EngineResult<Option<Vec<u8>>>
        where F: Read + Seek
    {
        self.regions.load_state(BDA_STATIC_HDR_SIZE, &mut f)
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

    /// The maximum size of variable length metadata that can be accommodated.
    pub fn max_data_size(&self) -> Sectors {
        self.regions.max_data_size()
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
        f.seek(SeekFrom::Start(0))?;
        let mut buf = [0u8; _BDA_STATIC_HDR_SIZE];
        f.read(&mut buf)?;

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

        let err_str = "Appeared to be a Stratis device, but no valid sigblock found";
        Err(EngineError::Engine(ErrorEnum::Invalid, err_str.into()))
    }

    /// Determine the ownership of a device.
    pub fn determine_ownership<F>(f: &mut F) -> EngineResult<DevOwnership>
        where F: Read + Seek
    {


        f.seek(SeekFrom::Start(0))?;
        let mut buf = [0u8; _BDA_STATIC_HDR_SIZE];
        f.read(&mut buf)?;

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
    fn sigblock_to_buf(&self) -> [u8; SECTOR_SIZE] {
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

        let pool_uuid = Uuid::parse_str(from_utf8(&buf[32..64])?)?;
        let dev_uuid = Uuid::parse_str(from_utf8(&buf[64..96])?)?;

        let mda_size = Sectors(LittleEndian::read_u64(&buf[96..104]));

        mda::validate_mda_size(mda_size)?;

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

mod mda {
    use std;
    use std::cmp::Ordering;
    use std::io::{Read, Seek, SeekFrom, Write};

    use byteorder::{ByteOrder, LittleEndian};
    use crc::crc32;
    use time::Timespec;

    use devicemapper::{Bytes, Sectors};

    use super::super::super::errors::{EngineResult, EngineError, ErrorEnum};

    const _MDA_REGION_HDR_SIZE: usize = 32;
    const MDA_REGION_HDR_SIZE: Bytes = Bytes(_MDA_REGION_HDR_SIZE as u64);

    const NUM_MDA_REGIONS: usize = 4;
    const PER_MDA_REGION_COPIES: usize = 2;
    const NUM_PRIMARY_MDA_REGIONS: usize = NUM_MDA_REGIONS / PER_MDA_REGION_COPIES;
    pub const MIN_MDA_SECTORS: Sectors = Sectors(2032);


    #[derive(Debug)]
    pub struct MDARegions {
        // Spec defines 4 regions, but regions 2 & 3 are duplicates of 0 and 1 respectively
        region_size: Sectors,
        mdas: [Option<MDAHeader>; NUM_PRIMARY_MDA_REGIONS],
    }

    impl MDARegions {
        /// Calculate the offset from start of device for an MDARegion.
        fn mda_offset(header_size: Bytes, index: usize, per_region_size: Bytes) -> u64 {
            *(header_size + per_region_size * index)
        }

        /// The maximum size of variable length metadata that this region
        /// can accommodate.
        pub fn max_data_size(&self) -> Sectors {
            self.region_size
        }

        /// Initialize the space allotted to the MDA regions to 0.
        /// Return an MDARegions object with uninitialized MDAHeader objects.
        pub fn initialize<F>(header_size: Bytes,
                             size: Sectors,
                             f: &mut F)
                             -> EngineResult<MDARegions>
            where F: Seek + Write
        {
            let hdr_buf = MDAHeader::default().to_buf();

            let region_size = size / NUM_MDA_REGIONS;
            let per_region_size = region_size.bytes();
            for region in 0..NUM_MDA_REGIONS {
                f.seek(SeekFrom::Start(MDARegions::mda_offset(header_size,
                                                                 region,
                                                                 per_region_size)))?;
                f.write_all(&hdr_buf)?;
            }

            f.flush()?;

            Ok(MDARegions {
                   region_size: region_size,
                   mdas: [None, None],
               })
        }

        /// Construct MDARegions from data on the disk.
        /// Note that this method is always called in a context where a
        /// StaticHeader has already been read. Therefore, it
        /// constitutes an error if it is not possible to discover two
        /// well-formed MDAHeaders for this device.
        pub fn load<F>(header_size: Bytes, size: Sectors, f: &mut F) -> EngineResult<MDARegions>
            where F: Read + Seek
        {
            let region_size = size / NUM_MDA_REGIONS;
            let per_region_size = region_size.bytes();

            // Load a single region at the location specified by index.
            // If it appears that no metadata has been written at the location
            // return None. If it appears that there is metadata, but it has
            // been corrrupted, return an error.
            let mut load_a_region = |index: usize| -> EngineResult<Option<MDAHeader>> {
                let mut hdr_buf = [0u8; _MDA_REGION_HDR_SIZE];
                f.seek(SeekFrom::Start(MDARegions::mda_offset(header_size,
                                                                 index,
                                                                 per_region_size)))?;
                f.read_exact(&mut hdr_buf)?;
                Ok(MDAHeader::from_buf(&hdr_buf, per_region_size)?)
            };

            // Get an MDAHeader for the given index.
            // If there is a failure reading the first, fall back on the
            // second. If there is a failure reading both, return an error.
            let mut get_mda = |index: usize| -> EngineResult<Option<MDAHeader>> {
                load_a_region(index).or_else(|_| load_a_region(index + 2))
            };

            Ok(MDARegions {
                   region_size: region_size,
                   mdas: [get_mda(0)?, get_mda(1)?],
               })
        }

        /// Write metadata to the older of the metadata regions.
        /// If operation is completed, update the value of the
        /// older MDAHeader with the new values.
        /// If time specified is earlier than the last update time, return an
        /// error. If the size of the data is greater than the available space,
        /// return an error. If there is an error when writing the data, return
        /// an error.
        pub fn save_state<F>(&mut self,
                             header_size: Bytes,
                             time: &Timespec,
                             data: &[u8],
                             f: &mut F)
                             -> EngineResult<()>
            where F: Seek + Write
        {
            if self.last_update_time() >= Some(time) {
                return Err(EngineError::Engine(ErrorEnum::Invalid,
                                               "Overwriting newer data".into()));
            }

            let region_size = self.region_size.bytes();
            let used = Bytes(data.len() as u64);
            check_mda_region_size(used, region_size)?;

            let header = MDAHeader {
                last_updated: *time,
                used: used,
                data_crc: crc32::checksum_ieee(data),
            };
            let hdr_buf = header.to_buf();

            // Write data to a region specified by index.
            let mut save_region = |index: usize| -> EngineResult<()> {
                f.seek(SeekFrom::Start(MDARegions::mda_offset(header_size, index, region_size)))?;
                f.write_all(&hdr_buf)?;
                f.write_all(data)?;
                f.flush()?;

                Ok(())
            };

            // TODO: Consider if there is an action that should be taken if
            // saving to one or the other region fails.
            let older_region = self.older();
            save_region(older_region)?;
            save_region(older_region + 2)?;

            self.mdas[older_region] = Some(header);

            Ok(())
        }

        /// Load metadata from the newer MDA region.
        /// In case there is no record of metadata in regions, return None.
        /// If there is a record of metadata, and there is a failure to read
        /// the metadata, return an error.
        pub fn load_state<F>(&self, header_size: Bytes, f: &mut F) -> EngineResult<Option<Vec<u8>>>
            where F: Read + Seek
        {
            let newer_region = self.newer();
            let mda = match self.mdas[newer_region] {
                None => return Ok(None),
                Some(ref mda) => mda,
            };
            let region_size = self.region_size.bytes();

            // Load the metadata region specified by index.
            // It is an error if the metadata can not be found.
            let mut load_region = |index: usize| -> EngineResult<Vec<u8>> {
                let offset = MDARegions::mda_offset(header_size, index, region_size) +
                             _MDA_REGION_HDR_SIZE as u64;
                f.seek(SeekFrom::Start(offset))?;
                mda.load_region(f)
            };

            // TODO: Figure out if there is an action to take if the
            // first read returns an error.
            load_region(newer_region)
                .or_else(|_| load_region(newer_region + 2))
                .map(Some)
        }

        /// The index of the older region, or 0 if there is a tie.
        fn older(&self) -> usize {
            match (&self.mdas[0], &self.mdas[1]) {
                (&None, _) => 0,
                (_, &None) => 1,
                (&Some(ref mda0), &Some(ref mda1)) => {
                    match mda0.last_updated.cmp(&mda1.last_updated) {
                        Ordering::Less => 0,
                        Ordering::Equal | Ordering::Greater => 1,
                    }
                }
            }
        }

        /// The index of the newer region, or 1 if there is a tie.
        fn newer(&self) -> usize {
            match self.older() {
                0 => 1,
                1 => 0,
                _ => panic!("invalid val from older()"),
            }
        }

        /// The last update time for these MDA regions
        pub fn last_update_time(&self) -> Option<&Timespec> {
            self.mdas[self.newer()].as_ref().map(|h| &h.last_updated)
        }
    }

    #[derive(Debug)]
    pub struct MDAHeader {
        last_updated: Timespec,

        /// Size of region used for pool metadata.
        used: Bytes,

        data_crc: u32,
    }

    // Implementing Default explicitly because Timespec does not implement
    // Default. The time crate has been superceded by the chrono crate, and
    // is in maintenance mode, so there is little point in submitting a PR
    // to change Timespec's behavior.
    impl Default for MDAHeader {
        fn default() -> MDAHeader {
            MDAHeader {
                last_updated: Timespec::new(0, 0),
                used: Bytes(0),
                data_crc: 0,
            }
        }
    }

    impl MDAHeader {
        /// Get an MDAHeader from the buffer.
        /// Return an error for a bad checksum.
        /// Return an error if the size of the region used is too large for the given region_size.
        /// Return None if there is no MDAHeader to be read. This is detected if the
        /// timestamp region in the buffer is 0.
        fn from_buf(buf: &[u8; _MDA_REGION_HDR_SIZE],
                    region_size: Bytes)
                    -> EngineResult<Option<MDAHeader>> {
            if LittleEndian::read_u32(&buf[..4]) != crc32::checksum_ieee(&buf[4..]) {
                return Err(EngineError::Engine(ErrorEnum::Invalid, "MDA region header CRC".into()));
            }

            match LittleEndian::read_u64(&buf[16..24]) {
                0 => Ok(None),
                secs => {
                    let used = Bytes(LittleEndian::read_u64(&buf[8..16]));
                    check_mda_region_size(used, region_size)?;

                    let nsecs = LittleEndian::read_u32(&buf[24..28]);
                    // Signed cast is safe, highest order bit of each value
                    // read is guaranteed to be 0.
                    assert!(nsecs <= std::i32::MAX as u32);
                    assert!(secs <= std::i64::MAX as u64);

                    Ok(Some(MDAHeader {
                                used: used,
                                last_updated: Timespec::new(secs as i64, nsecs as i32),
                                data_crc: LittleEndian::read_u32(&buf[4..8]),
                            }))
                }
            }
        }

        fn to_buf(&self) -> [u8; _MDA_REGION_HDR_SIZE] {
            // Unsigned casts are always safe, as sec and nsec values are never negative
            assert!(self.last_updated.sec >= 0 && self.last_updated.nsec >= 0);

            let mut buf = [0u8; _MDA_REGION_HDR_SIZE];

            LittleEndian::write_u32(&mut buf[4..8], self.data_crc);
            LittleEndian::write_u64(&mut buf[8..16], *self.used as u64);
            LittleEndian::write_u64(&mut buf[16..24], self.last_updated.sec as u64);
            LittleEndian::write_u32(&mut buf[24..28], self.last_updated.nsec as u32);

            let buf_crc = crc32::checksum_ieee(&buf[4.._MDA_REGION_HDR_SIZE]);
            LittleEndian::write_u32(&mut buf[..4], buf_crc);

            buf
        }

        /// Given a pre-seek()ed File, load the MDA region and return the contents.
        /// Return an error if the data can not be read, since the existance
        /// of the MDAHeader implies that the data must be available.
        // MDAHeader cannot seek because it doesn't know which region it's in
        fn load_region<F>(&self, f: &mut F) -> EngineResult<Vec<u8>>
            where F: Read
        {
            // This cast could fail if running on a 32-bit machine and
            // size of metadata is greater than 2^32 - 1 bytes, which is
            // unlikely.
            //
            // This comparison seems absurd when compiled in an environment
            // where usize is u64, which is usual. It is not absurd when
            // compiled in an environment where usize is u32.
            #![allow(absurd_extreme_comparisons)]
            assert!(*self.used <= std::usize::MAX as u64);
            let mut data_buf = vec![0u8; *self.used as usize];

            f.read_exact(&mut data_buf)?;

            if self.data_crc != crc32::checksum_ieee(&data_buf) {
                return Err(EngineError::Engine(ErrorEnum::Invalid, "MDA region data CRC".into()));
            }
            Ok(data_buf)
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

        use quickcheck::{QuickCheck, TestResult};
        use time::{now, Timespec};

        use super::super::*;
        use super::*;

        #[test]
        /// Verify that default MDAHeader is all 0s except for CRC.
        fn test_default_mda_header() {
            assert!(MDAHeader::default().to_buf()[4..].iter().all(|x| *x == 0u8));
        }

        #[test]
        /// Verify that loading the MDARegions fails if the regions are all 0s.
        /// Verify that loading MDARegions succeeds if the regions are properly
        /// initialized.
        fn test_reading_mda_regions() {
            let buf_length = *(BDA_STATIC_HDR_SIZE + 4usize * MIN_MDA_SECTORS.bytes()) as usize;
            let mut buf = Cursor::new(vec![0; buf_length]);
            assert!(MDARegions::load(BDA_STATIC_HDR_SIZE, MIN_MDA_SECTORS, &mut buf).is_err());

            MDARegions::initialize(BDA_STATIC_HDR_SIZE, MIN_MDA_SECTORS, &mut buf).unwrap();
            let regions = MDARegions::load(BDA_STATIC_HDR_SIZE, MIN_MDA_SECTORS, &mut buf).unwrap();
            assert!(regions.last_update_time().is_none());
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
                let region_size = (MIN_MDA_SECTORS / 4usize).bytes() +
                                  Bytes(region_size_ext as u64);
                let header = MDAHeader {
                    last_updated: Timespec::new(sec, nsec),
                    used: Bytes(data.len() as u64),
                    data_crc: crc32::checksum_ieee(&data),
                };
                let buf = header.to_buf();
                let mda1 = MDAHeader::from_buf(&buf, region_size).unwrap().unwrap();
                let mda2 = MDAHeader::from_buf(&buf, region_size).unwrap().unwrap();

                TestResult::from_bool(mda1.last_updated == mda2.last_updated &&
                                      mda1.used == mda2.used &&
                                      mda1.data_crc == mda2.data_crc &&
                                      header.last_updated == mda1.last_updated &&
                                      header.data_crc == mda1.data_crc)
            }

            QuickCheck::new()
                .tests(50)
                .quickcheck(mda_header as fn(Vec<u8>, i64, i32, u32) -> TestResult);
        }

        /// Verify that bad crc causes an error.
        #[test]
        fn test_from_buf_crc_error() {
            let data = [0u8; 3];
            let header = MDAHeader {
                last_updated: now().to_timespec(),
                used: Bytes(data.len() as u64),
                data_crc: crc32::checksum_ieee(&data),
            };
            let mut buf = header.to_buf();
            LittleEndian::write_u32(&mut buf[..4], 0u32);
            assert!(MDAHeader::from_buf(&buf, Bytes(data.len() as u64) + MDA_REGION_HDR_SIZE)
                        .is_err());
        }

        /// Verify that too small region_size causes an error.
        #[test]
        fn test_from_buf_size_error() {
            let data = [0u8; 3];
            let header = MDAHeader {
                last_updated: now().to_timespec(),
                used: Bytes(data.len() as u64),
                data_crc: crc32::checksum_ieee(&data),
            };
            let buf = header.to_buf();
            assert!(MDAHeader::from_buf(&buf, MDA_REGION_HDR_SIZE).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use devicemapper::{Bytes, Sectors};
    use quickcheck::{QuickCheck, TestResult};
    use time::now;
    use uuid::Uuid;

    use super::super::super::consts::IEC;

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
}
