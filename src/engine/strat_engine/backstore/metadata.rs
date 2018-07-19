// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};
use std::str::from_utf8;

use byteorder::{ByteOrder, LittleEndian};
use chrono::{DateTime, Utc};
use crc::crc32;
use uuid::Uuid;

use devicemapper::{Bytes, Sectors, IEC, SECTOR_SIZE};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::types::{DevUuid, PoolUuid};

use super::super::device::SyncAll;

pub use self::mda::{validate_mda_size, MIN_MDA_SECTORS};

const _BDA_STATIC_HDR_SIZE: usize = 16 * SECTOR_SIZE;
const BDA_STATIC_HDR_SIZE: Bytes = Bytes(_BDA_STATIC_HDR_SIZE as u64);

const MDA_RESERVED_SECTORS: Sectors = Sectors(3 * IEC::Mi / (SECTOR_SIZE as u64)); // = 3 MiB

const STRAT_MAGIC: &[u8] = b"!Stra0tis\x86\xff\x02^\x41rh";

#[derive(Debug)]
pub struct BDA {
    header: StaticHeader,
    regions: mda::MDARegions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MetadataLocation {
    Both,
    First,
    Second,
}

impl BDA {
    /// Read the BDA from the device and return 2 SECTORS worth of data, one for each BDA returned
    /// in the order of layout on disk (location 1, location 2).
    /// Only the BDA sectors are read up from disk, zero areas are *not* read.
    fn read<F>(f: &mut F) -> io::Result<([u8; SECTOR_SIZE], [u8; SECTOR_SIZE])>
    where
        F: Read + Seek,
    {
        // Theory of read procedure
        // We write the BDA in two operations with a sync in between.  The write operation
        // could fail (loss of power) for either write leaving sector(s) with potentially hard
        // read errors. It's best to read each of the specific BDA blocks individually, to limit
        // the probability of hitting a read error on a non-essential sector.

        let mut buf_loc_1 = [0u8; SECTOR_SIZE];
        let mut buf_loc_2 = [0u8; SECTOR_SIZE];

        /// Read a bda sector worth of data at the specified offset into buffer.
        fn read_sector_at_offset<F>(f: &mut F, offset: usize, mut buf: &mut [u8]) -> io::Result<()>
        where
            F: Read + Seek,
        {
            f.seek(SeekFrom::Start(offset as u64))?;
            f.read_exact(&mut buf)?;
            Ok(())
        }

        let loc_1_read_result = read_sector_at_offset(f, SECTOR_SIZE, &mut buf_loc_1);
        let loc_2_read_result = read_sector_at_offset(f, 9 * SECTOR_SIZE, &mut buf_loc_2);

        match (loc_1_read_result, loc_2_read_result) {
            (Err(loc_1_err), Err(_)) => Err(loc_1_err),
            _ => Ok((buf_loc_1, buf_loc_2)),
        }
    }

    // Writes bda_buf according to the value of which.
    // If first location is specified, write zeroes to empty regions in the
    // first 8 sectors. If the second location is specified, writes zeroes to empty
    // regions in the second 8 sectors.
    fn write<F>(f: &mut F, bda_buf: &[u8], which: MetadataLocation) -> io::Result<()>
    where
        F: Seek + SyncAll,
    {
        let zeroed = [0u8; _BDA_STATIC_HDR_SIZE];
        f.seek(SeekFrom::Start(0))?;

        // Write to a single region in the header. Zeroes the first sector,
        // writes bda_buf to the second sector, and then zeroes the remaining
        // six sectors.
        fn write_region<F>(f: &mut F, bda_buf: &[u8], zeroed: &[u8]) -> io::Result<()>
        where
            F: Seek + SyncAll,
        {
            f.write_all(&zeroed[..SECTOR_SIZE])?; // Zero 1 unused sector
            f.write_all(bda_buf)?;
            f.write_all(&zeroed[..SECTOR_SIZE * 6])?; // Zero 6 unused sectors
            f.sync_all()?;
            Ok(())
        };

        if which == MetadataLocation::Both || which == MetadataLocation::First {
            write_region(f, bda_buf, &zeroed)?;
        } else {
            f.seek(SeekFrom::Start(8 * SECTOR_SIZE as u64))?;
        }

        if which == MetadataLocation::Both || which == MetadataLocation::Second {
            write_region(f, bda_buf, &zeroed)?;
        }
        Ok(())
    }

    /// Initialize a blockdev with a Stratis BDA.
    pub fn initialize<F>(
        f: &mut F,
        pool_uuid: Uuid,
        dev_uuid: Uuid,
        mda_size: Sectors,
        blkdev_size: Sectors,
        initialization_time: u64,
    ) -> StratisResult<BDA>
    where
        F: Seek + SyncAll,
    {
        let header = StaticHeader::new(
            pool_uuid,
            dev_uuid,
            mda_size,
            blkdev_size,
            initialization_time,
        );

        BDA::write(f, &header.sigblock_to_buf(), MetadataLocation::Both)?;

        let regions = mda::MDARegions::initialize(BDA_STATIC_HDR_SIZE, header.mda_size, f)?;

        Ok(BDA { header, regions })
    }

    /// Load a BDA on initial setup of a device.
    /// Returns None if no BDA appears to exist.
    pub fn load<F>(f: &mut F) -> StratisResult<Option<BDA>>
    where
        F: Read + Seek + SyncAll,
    {
        let header = match StaticHeader::setup(f)? {
            Some(header) => header,
            None => return Ok(None),
        };

        let regions = mda::MDARegions::load(BDA_STATIC_HDR_SIZE, header.mda_size, f)?;

        Ok(Some(BDA { header, regions }))
    }

    /// Zero out Static Header on the blockdev. This causes it to no
    /// longer be seen as a Stratis blockdev.
    pub fn wipe<F>(f: &mut F) -> StratisResult<()>
    where
        F: Seek + SyncAll,
    {
        let zeroed = [0u8; _BDA_STATIC_HDR_SIZE];

        // Wiping Static Header should do it
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&zeroed)?;
        f.sync_all()?;
        Ok(())
    }

    /// Save metadata to the disk
    pub fn save_state<F>(
        &mut self,
        time: &DateTime<Utc>,
        metadata: &[u8],
        f: &mut F,
    ) -> StratisResult<()>
    where
        F: Seek + SyncAll,
    {
        self.regions
            .save_state(BDA_STATIC_HDR_SIZE, time, metadata, f)
    }

    /// Read latest metadata from the disk
    pub fn load_state<F>(&self, mut f: &mut F) -> StratisResult<Option<Vec<u8>>>
    where
        F: Read + Seek,
    {
        self.regions.load_state(BDA_STATIC_HDR_SIZE, &mut f)
    }

    /// The time when the most recent metadata was written to the BDA,
    /// if any.
    pub fn last_update_time(&self) -> Option<&DateTime<Utc>> {
        self.regions.last_update_time()
    }

    /// The UUID of the device.
    pub fn dev_uuid(&self) -> DevUuid {
        self.header.dev_uuid
    }

    /// The UUID of the device's pool.
    pub fn pool_uuid(&self) -> PoolUuid {
        self.header.pool_uuid
    }

    /// The size of the device.
    pub fn dev_size(&self) -> Sectors {
        self.header.blkdev_size
    }

    /// The number of sectors the BDA itself occupies.
    pub fn size(&self) -> Sectors {
        BDA_STATIC_HDR_SIZE.sectors() + self.header.mda_size + self.header.reserved_size
    }

    /// The maximum size of variable length metadata that can be accommodated.
    pub fn max_data_size(&self) -> Sectors {
        self.regions.max_data_size()
    }

    /// Timestamp when the device was initialized.
    pub fn initialization_time(&self) -> u64 {
        self.header.initialization_time
    }
}

#[derive(Eq, PartialEq)]
pub struct StaticHeader {
    blkdev_size: Sectors,
    pool_uuid: PoolUuid,
    dev_uuid: DevUuid,
    mda_size: Sectors,
    reserved_size: Sectors,
    flags: u64,
    /// Seconds portion of DateTime<Utc> value.
    initialization_time: u64,
}

impl StaticHeader {
    fn new(
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
        mda_size: Sectors,
        blkdev_size: Sectors,
        initialization_time: u64,
    ) -> StaticHeader {
        StaticHeader {
            blkdev_size,
            pool_uuid,
            dev_uuid,
            mda_size,
            reserved_size: MDA_RESERVED_SECTORS,
            flags: 0,
            initialization_time,
        }
    }

    /// Try to find a valid StaticHeader on a device.
    /// Return the latest copy that validates as a Stratis BDA, however verify both
    /// copies and if one validates but one does not, re-write the one that is incorrect.  If both
    /// copies are valid, but one is newer than the other, rewrite the older one to match.
    /// Return None if the static header's magic does not match for *both* copies.
    fn setup<F>(f: &mut F) -> StratisResult<Option<StaticHeader>>
    where
        F: Read + Seek + SyncAll,
    {
        let (buf_loc_1, buf_loc_2) = BDA::read(f)?;

        match (
            StaticHeader::sigblock_from_buf(&buf_loc_1),
            StaticHeader::sigblock_from_buf(&buf_loc_2),
        ) {
            (Ok(loc_1), Ok(loc_2)) => {
                match (loc_1, loc_2) {
                    (Some(loc_1), Some(loc_2)) => {
                        if loc_1 == loc_2 {
                            Ok(Some(loc_1))
                        } else if loc_1.initialization_time > loc_2.initialization_time {
                            BDA::write(f, &buf_loc_1, MetadataLocation::Second)?;
                            Ok(Some(loc_1))
                        } else {
                            BDA::write(f, &buf_loc_2, MetadataLocation::First)?;
                            Ok(Some(loc_2))
                        }
                    }
                    (None, None) => Ok(None),
                    (Some(loc_1), None) => {
                        // Copy 1 has valid Stratis BDA, copy 2 has no magic, re-write copy 2
                        BDA::write(f, &buf_loc_1, MetadataLocation::Second)?;
                        Ok(Some(loc_1))
                    }
                    (None, Some(loc_2)) => {
                        // Copy 2 has valid Stratis BDA, copy 1 has no magic, re-write copy 1
                        BDA::write(f, &buf_loc_2, MetadataLocation::First)?;
                        Ok(Some(loc_2))
                    }
                }
            }
            (Ok(loc_1), Err(loc_2)) => {
                // Re-write copy 2
                if loc_1.is_some() {
                    BDA::write(f, &buf_loc_1, MetadataLocation::Second)?;
                    Ok(loc_1)
                } else {
                    // Location 1 doesn't have a signature, but location 2 did, but it got an error,
                    // lets return the error instead as this appears to be a stratis device that
                    // has gotten in a bad state.
                    Err(loc_2)
                }
            }
            (Err(loc_1), Ok(loc_2)) => {
                // Re-write copy 1
                if loc_2.is_some() {
                    BDA::write(f, &buf_loc_2, MetadataLocation::First)?;
                    Ok(loc_2)
                } else {
                    // Location 2 doesn't have a signature, but location 1 did, but it got an error,
                    // lets return the error instead as this appears to be a stratis device that
                    // has gotten in a bad state.
                    Err(loc_1)
                }
            }
            (Err(_), Err(_)) => {
                let err_str = "Appeared to be a Stratis device, but no valid sigblock found";
                Err(StratisError::Engine(ErrorEnum::Invalid, err_str.into()))
            }
        }
    }

    /// Retrieve the device and pool UUIDs from a stratis device.
    pub fn device_identifiers<F>(f: &mut F) -> StratisResult<Option<((PoolUuid, DevUuid))>>
    where
        F: Read + Seek + SyncAll,
    {
        // Using setup() as a test of ownership sets a high bar. It is
        // not sufficient to have STRAT_MAGIC to be considered "Ours",
        // it must also have correct CRC, no weird stuff in fields,
        // etc!
        match StaticHeader::setup(f) {
            Ok(Some(sh)) => Ok(Some((sh.pool_uuid, sh.dev_uuid))),
            Ok(None) => Ok(None),
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
        LittleEndian::write_u64(&mut buf[120..128], self.initialization_time);

        let hdr_crc = crc32::checksum_castagnoli(&buf[4..SECTOR_SIZE]);
        LittleEndian::write_u32(&mut buf[..4], hdr_crc);
        buf
    }

    /// Build a StaticHeader from a SECTOR_SIZE buf that was read from
    /// a blockdev.
    fn sigblock_from_buf(buf: &[u8]) -> StratisResult<Option<StaticHeader>> {
        assert_eq!(buf.len(), SECTOR_SIZE);

        if &buf[4..20] != STRAT_MAGIC {
            return Ok(None);
        }

        let crc = crc32::checksum_castagnoli(&buf[4..SECTOR_SIZE]);
        if crc != LittleEndian::read_u32(&buf[..4]) {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                "header CRC invalid".into(),
            ));
        }

        let blkdev_size = Sectors(LittleEndian::read_u64(&buf[20..28]));

        let pool_uuid = Uuid::parse_str(from_utf8(&buf[32..64])?)?;
        let dev_uuid = Uuid::parse_str(from_utf8(&buf[64..96])?)?;

        let mda_size = Sectors(LittleEndian::read_u64(&buf[96..104]));

        mda::validate_mda_size(mda_size)?;

        Ok(Some(StaticHeader {
            pool_uuid,
            dev_uuid,
            blkdev_size,
            mda_size,
            reserved_size: Sectors(LittleEndian::read_u64(&buf[104..112])),
            flags: 0,
            initialization_time: LittleEndian::read_u64(&buf[120..128]),
        }))
    }
}

impl fmt::Debug for StaticHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("StaticHeader")
            .field("blkdev_size", &self.blkdev_size)
            .field("pool_uuid", &self.pool_uuid.simple().to_string())
            .field("dev_uuid", &self.dev_uuid.simple().to_string())
            .field("mda_size", &self.mda_size)
            .field("reserved_size", &self.reserved_size)
            .field("flags", &self.flags)
            .field("initialization_time", &self.initialization_time)
            .finish()
    }
}

mod mda {
    use std;
    use std::cmp::Ordering;
    use std::io::{Read, Seek, SeekFrom};

    use byteorder::{ByteOrder, LittleEndian};
    use chrono::{DateTime, TimeZone, Utc};
    use crc::crc32;

    use devicemapper::{Bytes, Sectors};

    use stratis::{ErrorEnum, StratisError, StratisResult};

    use super::SyncAll;

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
        pub fn initialize<F>(
            header_size: Bytes,
            size: Sectors,
            f: &mut F,
        ) -> StratisResult<MDARegions>
        where
            F: Seek + SyncAll,
        {
            let hdr_buf = MDAHeader::default().to_buf();

            let region_size = size / NUM_MDA_REGIONS;
            let per_region_size = region_size.bytes();
            for region in 0..NUM_MDA_REGIONS {
                f.seek(SeekFrom::Start(MDARegions::mda_offset(
                    header_size,
                    region,
                    per_region_size,
                )))?;
                f.write_all(&hdr_buf)?;
            }

            f.sync_all()?;

            Ok(MDARegions {
                region_size,
                mdas: [None, None],
            })
        }

        /// Construct MDARegions from data on the disk.
        /// Note that this method is always called in a context where a
        /// StaticHeader has already been read. Therefore, it
        /// constitutes an error if it is not possible to discover two
        /// well-formed MDAHeaders for this device.
        pub fn load<F>(header_size: Bytes, size: Sectors, f: &mut F) -> StratisResult<MDARegions>
        where
            F: Read + Seek,
        {
            let region_size = size / NUM_MDA_REGIONS;
            let per_region_size = region_size.bytes();

            // Load a single region at the location specified by index.
            // If it appears that no metadata has been written at the location
            // return None. If it appears that there is metadata, but it has
            // been corrupted, return an error.
            let mut load_a_region = |index: usize| -> StratisResult<Option<MDAHeader>> {
                let mut hdr_buf = [0u8; _MDA_REGION_HDR_SIZE];
                f.seek(SeekFrom::Start(MDARegions::mda_offset(
                    header_size,
                    index,
                    per_region_size,
                )))?;
                f.read_exact(&mut hdr_buf)?;
                Ok(MDAHeader::from_buf(&hdr_buf, per_region_size)?)
            };

            // Get an MDAHeader for the given index.
            // If there is a failure reading the first, fall back on the
            // second. If there is a failure reading both, return an error.
            let mut get_mda = |index: usize| -> StratisResult<Option<MDAHeader>> {
                load_a_region(index).or_else(|_| load_a_region(index + 2))
            };

            Ok(MDARegions {
                region_size,
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
        pub fn save_state<F>(
            &mut self,
            header_size: Bytes,
            time: &DateTime<Utc>,
            data: &[u8],
            f: &mut F,
        ) -> StratisResult<()>
        where
            F: Seek + SyncAll,
        {
            if self.last_update_time() >= Some(time) {
                return Err(StratisError::Engine(
                    ErrorEnum::Invalid,
                    "Overwriting newer data".into(),
                ));
            }

            let region_size = self.region_size.bytes();
            let used = Bytes(data.len() as u64);
            check_mda_region_size(used, region_size)?;

            let header = MDAHeader {
                last_updated: *time,
                used,
                data_crc: crc32::checksum_castagnoli(data),
            };
            let hdr_buf = header.to_buf();

            // Write data to a region specified by index.
            let mut save_region = |index: usize| -> StratisResult<()> {
                f.seek(SeekFrom::Start(MDARegions::mda_offset(
                    header_size,
                    index,
                    region_size,
                )))?;
                f.write_all(&hdr_buf)?;
                f.write_all(data)?;
                f.sync_all()?;

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
        pub fn load_state<F>(&self, header_size: Bytes, f: &mut F) -> StratisResult<Option<Vec<u8>>>
        where
            F: Read + Seek,
        {
            let newer_region = self.newer();
            let mda = match self.mdas[newer_region] {
                None => return Ok(None),
                Some(ref mda) => mda,
            };
            let region_size = self.region_size.bytes();

            // Load the metadata region specified by index.
            // It is an error if the metadata can not be found.
            let mut load_region = |index: usize| -> StratisResult<Vec<u8>> {
                let offset = MDARegions::mda_offset(header_size, index, region_size)
                    + _MDA_REGION_HDR_SIZE as u64;
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
        pub fn last_update_time(&self) -> Option<&DateTime<Utc>> {
            self.mdas[self.newer()].as_ref().map(|h| &h.last_updated)
        }
    }

    #[derive(Debug)]
    pub struct MDAHeader {
        last_updated: DateTime<Utc>,

        /// Size of region used for pool metadata.
        used: Bytes,

        data_crc: u32,
    }

    // Implementing Default explicitly because DateTime<Utc> does not implement
    // Default.
    impl Default for MDAHeader {
        fn default() -> MDAHeader {
            MDAHeader {
                last_updated: Utc.timestamp(0, 0),
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
        fn from_buf(
            buf: &[u8; _MDA_REGION_HDR_SIZE],
            region_size: Bytes,
        ) -> StratisResult<Option<MDAHeader>> {
            if LittleEndian::read_u32(&buf[..4]) != crc32::checksum_castagnoli(&buf[4..]) {
                return Err(StratisError::Engine(
                    ErrorEnum::Invalid,
                    "MDA region header CRC".into(),
                ));
            }

            match LittleEndian::read_u64(&buf[16..24]) {
                0 => Ok(None),
                secs => {
                    let used = Bytes(LittleEndian::read_u64(&buf[8..16]));
                    check_mda_region_size(used, region_size)?;

                    // Signed cast is safe, highest order bit of each value
                    // read is guaranteed to be 0.
                    assert!(secs <= std::i64::MAX as u64);

                    let nsecs = LittleEndian::read_u32(&buf[24..28]);
                    Ok(Some(MDAHeader {
                        used,
                        last_updated: Utc.timestamp(secs as i64, nsecs),
                        data_crc: LittleEndian::read_u32(&buf[4..8]),
                    }))
                }
            }
        }

        fn to_buf(&self) -> [u8; _MDA_REGION_HDR_SIZE] {
            // Unsigned casts are always safe, as sec and nsec values are never negative
            assert!(self.last_updated.timestamp() >= 0);

            let mut buf = [0u8; _MDA_REGION_HDR_SIZE];

            LittleEndian::write_u32(&mut buf[4..8], self.data_crc);
            LittleEndian::write_u64(&mut buf[8..16], *self.used as u64);
            LittleEndian::write_u64(&mut buf[16..24], self.last_updated.timestamp() as u64);
            LittleEndian::write_u32(&mut buf[24..28], self.last_updated.timestamp_subsec_nanos());

            let buf_crc = crc32::checksum_castagnoli(&buf[4.._MDA_REGION_HDR_SIZE]);
            LittleEndian::write_u32(&mut buf[..4], buf_crc);

            buf
        }

        /// Given a pre-seek()ed File, load the MDA region and return the contents.
        /// Return an error if the data can not be read, since the existence
        /// of the MDAHeader implies that the data must be available.
        // MDAHeader cannot seek because it doesn't know which region it's in
        fn load_region<F>(&self, f: &mut F) -> StratisResult<Vec<u8>>
        where
            F: Read,
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

            if self.data_crc != crc32::checksum_castagnoli(&data_buf) {
                return Err(StratisError::Engine(
                    ErrorEnum::Invalid,
                    "MDA region data CRC".into(),
                ));
            }
            Ok(data_buf)
        }
    }

    /// Check that data size does not exceed region available.
    /// Note that used is the amount used for metadata only.
    fn check_mda_region_size(used: Bytes, available: Bytes) -> StratisResult<()> {
        if MDA_REGION_HDR_SIZE + used > available {
            let err_msg = format!(
                "metadata length {} exceeds region available {}",
                used,
                // available region > header size
                available - MDA_REGION_HDR_SIZE
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
        };
        Ok(())
    }

    /// Validate MDA size
    pub fn validate_mda_size(size: Sectors) -> StratisResult<()> {
        if size % NUM_MDA_REGIONS != Sectors(0) {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!(
                    "MDA size {} is not divisible by number of \
                     copies required {}",
                    size, NUM_MDA_REGIONS
                ),
            ));
        };

        if size < MIN_MDA_SECTORS {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!(
                    "MDA size {} is less than minimum ({})",
                    size, MIN_MDA_SECTORS
                ),
            ));
        };
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use std::io::Cursor;

        use chrono::Utc;
        use quickcheck::{QuickCheck, TestResult};

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
            fn mda_header(data: Vec<u8>, sec: i64, nsec: u32, region_size_ext: u32) -> TestResult {
                // sec < 0: unwritable timestamp
                // sec == 0: value of 0 is interpreted as no timestamp when read
                if sec <= 0 {
                    return TestResult::discard();
                }

                // 4 is NUM_MDA_REGIONS which is not imported from super.
                let region_size =
                    (MIN_MDA_SECTORS / 4usize).bytes() + Bytes(region_size_ext as u64);
                let header = MDAHeader {
                    last_updated: Utc.timestamp(sec, nsec),
                    used: Bytes(data.len() as u64),
                    data_crc: crc32::checksum_castagnoli(&data),
                };
                let buf = header.to_buf();
                let mda1 = MDAHeader::from_buf(&buf, region_size).unwrap().unwrap();
                let mda2 = MDAHeader::from_buf(&buf, region_size).unwrap().unwrap();

                TestResult::from_bool(
                    mda1.last_updated == mda2.last_updated
                        && mda1.used == mda2.used
                        && mda1.data_crc == mda2.data_crc
                        && header.last_updated == mda1.last_updated
                        && header.data_crc == mda1.data_crc,
                )
            }

            QuickCheck::new()
                .tests(50)
                .quickcheck(mda_header as fn(Vec<u8>, i64, u32, u32) -> TestResult);
        }

        /// Verify that bad crc causes an error.
        #[test]
        fn test_from_buf_crc_error() {
            let data = [0u8; 3];
            let header = MDAHeader {
                last_updated: Utc::now(),
                used: Bytes(data.len() as u64),
                data_crc: crc32::checksum_castagnoli(&data),
            };
            let mut buf = header.to_buf();
            LittleEndian::write_u32(&mut buf[..4], 0u32);
            assert!(
                MDAHeader::from_buf(&buf, Bytes(data.len() as u64) + MDA_REGION_HDR_SIZE).is_err()
            );
        }

        /// Verify that too small region_size causes an error.
        #[test]
        fn test_from_buf_size_error() {
            let data = [0u8; 3];
            let header = MDAHeader {
                last_updated: Utc::now(),
                used: Bytes(data.len() as u64),
                data_crc: crc32::checksum_castagnoli(&data),
            };
            let buf = header.to_buf();
            assert!(MDAHeader::from_buf(&buf, MDA_REGION_HDR_SIZE).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use devicemapper::{Bytes, Sectors, IEC};
    use quickcheck::{QuickCheck, TestResult};
    use uuid::Uuid;

    use super::*;

    /// Corrupt a byte at the specified position.
    fn corrupt_byte<F>(f: &mut F, position: u64) -> io::Result<()>
    where
        F: Read + Seek + SyncAll,
    {
        let mut byte_to_corrupt = [0; 1];
        f.seek(SeekFrom::Start(position))?;
        f.read(&mut byte_to_corrupt)?;
        byte_to_corrupt[0] = !byte_to_corrupt[0];
        f.seek(SeekFrom::Start(position))?;
        f.write(&byte_to_corrupt)?;
        f.sync_all()?;
        Ok(())
    }

    /// Return a static header with random block device and MDA size.
    /// The block device is less than the minimum, for efficiency in testing.
    fn random_static_header(blkdev_size: u64, mda_size_factor: u32) -> StaticHeader {
        let pool_uuid = Uuid::new_v4();
        let dev_uuid = Uuid::new_v4();
        let mda_size = MIN_MDA_SECTORS + Sectors((mda_size_factor * 4) as u64);
        let blkdev_size = (Bytes(IEC::Mi) + Sectors(blkdev_size).bytes()).sectors();
        StaticHeader::new(
            pool_uuid,
            dev_uuid,
            mda_size,
            blkdev_size,
            Utc::now().timestamp() as u64,
        )
    }

    #[test]
    /// Construct an arbitrary StaticHeader object.
    /// Verify that the "memory buffer" is unowned.
    /// Initialize a BDA.
    /// Verify that Stratis buffer validates.
    /// Wipe the BDA.
    /// Verify that the buffer is again unowned.
    fn prop_test_ownership() {
        fn test_ownership(blkdev_size: u64, mda_size_factor: u32) -> TestResult {
            let sh = random_static_header(blkdev_size, mda_size_factor);
            let buf_size = *sh.mda_size.bytes() as usize + _BDA_STATIC_HDR_SIZE;
            let mut buf = Cursor::new(vec![0; buf_size]);
            let ownership = StaticHeader::device_identifiers(&mut buf).unwrap();
            if ownership.is_some() {
                return TestResult::failed();
            }

            BDA::initialize(
                &mut buf,
                sh.pool_uuid,
                sh.dev_uuid,
                sh.mda_size,
                sh.blkdev_size,
                Utc::now().timestamp() as u64,
            ).unwrap();
            if let Some((t_p, t_d)) = StaticHeader::device_identifiers(&mut buf).unwrap() {
                if t_p != sh.pool_uuid || t_d != sh.dev_uuid {
                    return TestResult::failed();
                }
            } else {
                return TestResult::failed();
            }

            BDA::wipe(&mut buf).unwrap();
            let ownership = StaticHeader::device_identifiers(&mut buf).unwrap();
            if ownership.is_some() {
                return TestResult::failed();
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
            let buf_size = *sh.mda_size.bytes() as usize + _BDA_STATIC_HDR_SIZE;
            let mut buf = Cursor::new(vec![0; buf_size]);
            let bda = BDA::initialize(
                &mut buf,
                sh.pool_uuid,
                sh.dev_uuid,
                sh.mda_size,
                sh.blkdev_size,
                Utc::now().timestamp() as u64,
            ).unwrap();
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
        let mut bda = BDA::initialize(
            &mut buf,
            sh.pool_uuid,
            sh.dev_uuid,
            sh.mda_size,
            sh.blkdev_size,
            Utc::now().timestamp() as u64,
        ).unwrap();

        let timestamp0 = Utc::now();
        let timestamp1 = Utc::now();
        assert_ne!(timestamp0, timestamp1);

        let mut buf = Cursor::new(vec![0; *sh.blkdev_size.bytes() as usize]);
        bda.save_state(&timestamp1, &data, &mut buf).unwrap();

        // Error, because current timestamp is older than written to newer.
        assert!(bda.save_state(&timestamp0, &data, &mut buf).is_err());

        let timestamp2 = Utc::now();
        let timestamp3 = Utc::now();
        assert_ne!(timestamp2, timestamp3);

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
        fn check_state(
            blkdev_size: u64,
            mda_size_factor: u32,
            state: Vec<u8>,
            next_state: Vec<u8>,
        ) -> TestResult {
            let sh = random_static_header(blkdev_size, mda_size_factor);
            let buf_size = *sh.mda_size.bytes() as usize + _BDA_STATIC_HDR_SIZE;
            let mut buf = Cursor::new(vec![0; buf_size]);
            let mut bda = BDA::initialize(
                &mut buf,
                sh.pool_uuid,
                sh.dev_uuid,
                sh.mda_size,
                sh.blkdev_size,
                Utc::now().timestamp() as u64,
            ).unwrap();
            let current_time = Utc::now();
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

            let current_time = Utc::now();
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
            TestResult::from_bool(
                sh1.pool_uuid == sh2.pool_uuid
                    && sh1.dev_uuid == sh2.dev_uuid
                    && sh1.blkdev_size == sh2.blkdev_size
                    && sh1.mda_size == sh2.mda_size
                    && sh1.reserved_size == sh2.reserved_size
                    && sh1.flags == sh2.flags
                    && sh1.initialization_time == sh2.initialization_time,
            )
        }

        QuickCheck::new()
            .tests(30)
            .quickcheck(static_header as fn(u64, u32) -> TestResult);
    }

    #[test]
    /// Verify that we correctly handle reading up the BDA when we walk a byte of corruption through
    /// the BDA sector for each copy and correct the corrupt copy and return an error when
    /// we corrupt both copies.
    fn bda_test_recovery() {
        let sh = random_static_header(10000, 4);
        let buf_size = *sh.mda_size.bytes() as usize + _BDA_STATIC_HDR_SIZE;
        let mut buf = Cursor::new(vec![0; buf_size]);
        BDA::initialize(
            &mut buf,
            sh.pool_uuid,
            sh.dev_uuid,
            sh.mda_size,
            sh.blkdev_size,
            Utc::now().timestamp() as u64,
        ).unwrap();

        let reference_buf = buf.clone();

        for i in 0..SECTOR_SIZE {
            for primary in &[true, false] {
                for secondary in &[true, false] {
                    if !*primary {
                        // Corrupt primary copy
                        corrupt_byte(&mut buf, (1 * SECTOR_SIZE + i) as u64).unwrap();
                    }

                    if !*secondary {
                        // Corrupt secondary copy
                        corrupt_byte(&mut buf, (9 * SECTOR_SIZE + i) as u64).unwrap();
                    }

                    let setup_result = StaticHeader::setup(&mut buf);

                    if *primary || *secondary {
                        // Setup should work and buffer should be corrected
                        assert!(setup_result.is_ok() && setup_result.unwrap().is_some());

                        // Check buffer, should be corrected.
                        assert_eq!(reference_buf.get_ref(), buf.get_ref());
                    } else {
                        // Setup should fail to find a usable Stratis BDA
                        match i {
                            4...19 => {
                                // When we corrupt the magics then we believe that the signature is
                                // not ours and will return Ok(None)
                                assert!(setup_result.is_ok() && setup_result.unwrap().is_none());
                            }
                            _ => {
                                assert!(setup_result.is_err());
                            }
                        }

                        // Check buffer, should be different
                        {
                            assert_ne!(reference_buf.get_ref(), buf.get_ref());
                        }

                        // Reset the buffer as we didn't correct it
                        buf = reference_buf.clone();
                    }
                }
            }
        }
    }

    #[test]
    /// Test that we re-write the older of two BDAs if they don't match.
    fn bda_test_rewrite_older() {
        let sh = random_static_header(10000, 4);
        let buf_size = *sh.mda_size.bytes() as usize + _BDA_STATIC_HDR_SIZE;
        let mut buf = Cursor::new(vec![0; buf_size]);
        let ts = Utc::now().timestamp() as u64;

        BDA::initialize(
            &mut buf,
            sh.pool_uuid,
            sh.dev_uuid,
            sh.mda_size,
            sh.blkdev_size,
            ts,
        ).unwrap();

        let mut buf_newer = Cursor::new(vec![0; buf_size]);
        BDA::initialize(
            &mut buf_newer,
            sh.pool_uuid,
            sh.dev_uuid,
            sh.mda_size,
            sh.blkdev_size,
            ts + 1,
        ).unwrap();

        // We should always match this reference buffer as it's the newer one.
        let reference_buf = buf_newer.clone();

        for offset in &[SECTOR_SIZE, 9 * SECTOR_SIZE] {
            // Copy the older BDA to newer BDA buffer
            buf.seek(SeekFrom::Start(*offset as u64)).unwrap();
            buf_newer.seek(SeekFrom::Start(*offset as u64)).unwrap();
            let mut sector = [0u8; SECTOR_SIZE];
            buf.read_exact(&mut sector).unwrap();
            buf_newer.write_all(&sector).unwrap();

            assert_ne!(reference_buf.get_ref(), buf_newer.get_ref());

            let setup_result = StaticHeader::setup(&mut buf_newer);
            assert!(setup_result.is_ok() && setup_result.unwrap().is_some());

            // We should match the reference buffer
            assert_eq!(reference_buf.get_ref(), buf_newer.get_ref());
        }
    }

    #[test]
    /// Test that we get an error and not Ok(None) when one copy is missing a valid signature
    /// and the other copy fails (eg. CRC).
    fn bda_none_and_err() {
        let sh = random_static_header(10000, 4);
        let buf_size = *sh.mda_size.bytes() as usize + _BDA_STATIC_HDR_SIZE;
        let mut buf = Cursor::new(vec![0; buf_size]);

        BDA::initialize(
            &mut buf,
            sh.pool_uuid,
            sh.dev_uuid,
            sh.mda_size,
            sh.blkdev_size,
            Utc::now().timestamp() as u64,
        ).unwrap();

        // Corrupt magic in copy 1 to induce Ok(None)
        corrupt_byte(&mut buf, (SECTOR_SIZE + 10) as u64).unwrap();

        // Corrupt a non-magic byte in copy 2 to induce Err()
        corrupt_byte(&mut buf, (SECTOR_SIZE * 9 + 128) as u64).unwrap();

        assert!(StaticHeader::setup(&mut buf).is_err());
    }
}
