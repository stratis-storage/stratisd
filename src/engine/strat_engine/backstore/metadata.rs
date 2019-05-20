// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fmt,
    io::{self, Read, Seek, SeekFrom},
    str::from_utf8,
};

use byteorder::{ByteOrder, LittleEndian};
use chrono::{DateTime, Utc};
use crc::crc32;
use uuid::Uuid;

use devicemapper::{Bytes, Sectors, IEC, SECTOR_SIZE};

use crate::{
    engine::{strat_engine::device::SyncAll, DevUuid, PoolUuid},
    stratis::{ErrorEnum, StratisError, StratisResult},
};

pub use self::mda::MDADataSize;

const _BDA_STATIC_HDR_SIZE: usize = 16 * SECTOR_SIZE;
const BDA_STATIC_HDR_SIZE: Bytes = Bytes(_BDA_STATIC_HDR_SIZE as u64);

const MDA_RESERVED_SECTORS: Sectors = Sectors(3 * IEC::Mi / (SECTOR_SIZE as u64)); // = 3 MiB

const STRAT_MAGIC: &[u8] = b"!Stra0tis\x86\xff\x02^\x41rh";

const STRAT_SIGBLOCK_VERSION: u8 = 1;

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
    fn read<F>(f: &mut F) -> (io::Result<[u8; SECTOR_SIZE]>, io::Result<[u8; SECTOR_SIZE]>)
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

        (
            read_sector_at_offset(f, SECTOR_SIZE, &mut buf_loc_1).map(|_| buf_loc_1),
            read_sector_at_offset(f, 9 * SECTOR_SIZE, &mut buf_loc_2).map(|_| buf_loc_2),
        )
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
        mda_data_size: mda::MDADataSize,
        blkdev_size: Sectors,
        initialization_time: u64,
    ) -> StratisResult<BDA>
    where
        F: Seek + SyncAll,
    {
        let header = StaticHeader::new(
            pool_uuid,
            dev_uuid,
            mda_data_size.region_size().mda_size(),
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
        BDA_STATIC_HDR_SIZE.sectors() + self.header.mda_size.sectors() + self.header.reserved_size
    }

    /// The maximum size of variable length metadata that can be accommodated.
    pub fn max_data_size(&self) -> MDADataSize {
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
    mda_size: mda::MDASize,
    reserved_size: Sectors,
    flags: u64,
    /// Seconds portion of DateTime<Utc> value.
    initialization_time: u64,
}

impl StaticHeader {
    fn new(
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
        mda_size: mda::MDASize,
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
    /// Return None if it's not a Stratis device.
    /// Return an error if the metadata seems to indicate that the device is
    /// a Stratis device, but no well-formed signature block could be read.
    /// Return an error if neither sigblock location can be read.
    /// Return an error if the sigblocks differ in some unaccountable way.
    /// Returns an error if a write intended to repair an ill-formed,
    /// unreadable, or stale signature block failed.
    fn setup<F>(f: &mut F) -> StratisResult<Option<StaticHeader>>
    where
        F: Read + Seek + SyncAll,
    {
        match BDA::read(f) {
            (Ok(buf_loc_1), Ok(buf_loc_2)) => {
                // We read both copies without an IO error.
                match (
                    StaticHeader::sigblock_from_buf(&buf_loc_1),
                    StaticHeader::sigblock_from_buf(&buf_loc_2),
                ) {
                    (Ok(loc_1), Ok(loc_2)) => {
                        match (loc_1, loc_2) {
                            (Some(loc_1), Some(loc_2)) => {
                                if loc_1 == loc_2 {
                                    Ok(Some(loc_1))
                                } else if loc_1.initialization_time == loc_2.initialization_time {
                                    // Inexplicable disagreement among static headers
                                    let err_str = "Appeared to be a Stratis device, but signature blocks disagree.";
                                    Err(StratisError::Engine(ErrorEnum::Invalid, err_str.into()))
                                } else if loc_1.initialization_time > loc_2.initialization_time {
                                    // If the first header block is newer, overwrite second with
                                    // contents of first.
                                    BDA::write(f, &buf_loc_1, MetadataLocation::Second)?;
                                    Ok(Some(loc_1))
                                } else {
                                    // The second header block must be newer, so overwrite first
                                    // with contents of second.
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
                        let err_str =
                            "Appeared to be a Stratis device, but no valid sigblock found";
                        Err(StratisError::Engine(ErrorEnum::Invalid, err_str.into()))
                    }
                }
            }
            (Ok(buf_loc_1), Err(_)) => {
                // Copy 1 read OK, 2 resulted in an IO error
                match StaticHeader::sigblock_from_buf(&buf_loc_1) {
                    Ok(loc_1) => {
                        if loc_1.is_some() {
                            BDA::write(f, &buf_loc_1, MetadataLocation::Second)?;
                        }
                        Ok(loc_1)
                    }
                    Err(e) => {
                        // Unable to determine if location 2 has a signature, but location 1 did,
                        // but it got an error, lets return the error instead as this appears to
                        // be a stratis device that has gotten in a bad state.
                        Err(e)
                    }
                }
            }
            (Err(_), Ok(buf_loc_2)) => {
                // Copy 2 read OK, 1 resulted in IO Error
                match StaticHeader::sigblock_from_buf(&buf_loc_2) {
                    Ok(loc_2) => {
                        if loc_2.is_some() {
                            BDA::write(f, &buf_loc_2, MetadataLocation::First)?;
                        }
                        Ok(loc_2)
                    }
                    Err(e) => {
                        // Unable to determine if location 1 has a signature, but location 2 did,
                        // but it got an error, lets return the error instead as this appears to
                        // be a stratis device that has gotten in a bad state.
                        Err(e)
                    }
                }
            }
            (Err(_), Err(_)) => {
                // Unable to read the device at all.
                let err_str = "Unable to read data at sigblock locations.";
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
        buf[28] = STRAT_SIGBLOCK_VERSION;
        buf[32..64].clone_from_slice(self.pool_uuid.to_simple_ref().to_string().as_bytes());
        buf[64..96].clone_from_slice(self.dev_uuid.to_simple_ref().to_string().as_bytes());
        LittleEndian::write_u64(&mut buf[96..104], *self.mda_size.sectors());
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

        let version = buf[28];
        if version != STRAT_SIGBLOCK_VERSION {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!("Unknown sigblock version: {}", version),
            ));
        }

        let pool_uuid = Uuid::parse_str(from_utf8(&buf[32..64])?)?;
        let dev_uuid = Uuid::parse_str(from_utf8(&buf[64..96])?)?;

        let mda_size = mda::MDASize(Sectors(LittleEndian::read_u64(&buf[96..104])));

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
            .field("pool_uuid", &self.pool_uuid.to_simple_ref())
            .field("dev_uuid", &self.dev_uuid.to_simple_ref())
            .field("mda_size", &self.mda_size)
            .field("reserved_size", &self.reserved_size)
            .field("flags", &self.flags)
            .field("initialization_time", &self.initialization_time)
            .finish()
    }
}

mod mda {
    use std::{
        self,
        cmp::Ordering,
        io::{Read, Seek, SeekFrom},
    };

    use byteorder::{ByteOrder, LittleEndian};
    use chrono::{DateTime, TimeZone, Utc};
    use crc::crc32;

    use devicemapper::Bytes;

    use crate::{
        engine::strat_engine::device::SyncAll,
        stratis::{ErrorEnum, StratisError, StratisResult},
    };

    pub use mda_size::{MDADataSize, MDARegionSize, MDASize, MIN_MDA_DATA_REGION_SIZE};

    const STRAT_REGION_HDR_VERSION: u8 = 1;
    const STRAT_METADATA_VERSION: u8 = 1;

    /// A module which defines types for three different regions of the MDA:
    /// * MDADataSize: the size of the region for variable length metadata
    /// * MDARegionSize: the size a single MDA region
    /// * MDASize: the size of the whole MDA
    mod mda_size {

        use devicemapper::{Bytes, Sectors};

        pub const _MDA_REGION_HDR_SIZE: usize = 32;
        const MDA_REGION_HDR_SIZE: Bytes = Bytes(_MDA_REGION_HDR_SIZE as u64);

        // The minimum size allocated for variable length metadata
        pub const MIN_MDA_DATA_REGION_SIZE: Bytes = Bytes(260_064);

        pub const NUM_PRIMARY_MDA_REGIONS: usize = 2;

        // There are two copies of every primary MDA region, so the total number
        // of MDA regions is twice the number of primary MDA regions.
        pub const NUM_MDA_REGIONS: usize = 2 * NUM_PRIMARY_MDA_REGIONS;

        /// A value representing the size of the entire MDA.
        /// It is constructed in one of two ways:
        /// * By reading a value from a device in constructing a StaticHeader
        /// * MDARegionSize::mda_size
        /// Since only a valid MDASize can be constructed, only a valid MDASize
        /// can be written. An error on reading ought to be detected by
        /// checksums.
        /// Since MDARegionSize is always at least the minimum, the result of
        /// MDARegionSize::mda_size is at least the minimum. The method, by
        /// definition, constructs a valid MDASize value.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct MDASize(pub Sectors);

        impl Default for MDASize {
            fn default() -> MDASize {
                MDARegionSize::default().mda_size()
            }
        }

        impl MDASize {
            pub fn sectors(self) -> Sectors {
                self.0
            }

            pub fn region_size(self) -> MDARegionSize {
                MDARegionSize(self.0 / NUM_MDA_REGIONS)
            }
        }

        /// A value representing the size of one MDA region.
        /// Values of this type are created by one of two methods:
        /// * MDASize::region_size
        /// * MDADataSize::region_size
        /// Since an MDADataSize is always at least the minimum required by the
        /// design specification, MDADataSize::region_size() always yields a
        /// value of at least the minimum required size.
        /// Since an MDASize is always valid, and at least the minimum,
        /// MDASize::region_size() always yields a valid and sufficiently large
        /// region.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct MDARegionSize(pub Sectors);

        impl Default for MDARegionSize {
            fn default() -> MDARegionSize {
                MDADataSize::default().region_size()
            }
        }

        impl MDARegionSize {
            pub fn sectors(self) -> Sectors {
                self.0
            }

            pub fn mda_size(self) -> MDASize {
                MDASize(self.0 * NUM_MDA_REGIONS)
            }

            pub fn data_size(self) -> MDADataSize {
                MDADataSize(self.0.bytes() - MDA_REGION_HDR_SIZE)
            }
        }

        /// A type representing the size of the region for storing variable length
        /// metadata. A newly created value is never less than the minimum required
        /// by the design specification.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct MDADataSize(Bytes);

        impl Default for MDADataSize {
            fn default() -> MDADataSize {
                MDADataSize(MIN_MDA_DATA_REGION_SIZE)
            }
        }

        impl MDADataSize {
            /// Create a new value, bounded from below by the minimum allowed.
            // Note that this code is dead due to GitHub issue:
            // https://github.com/stratis-storage/stratisd/issues/754.
            // To fix that bug it is necessary for client code to specify a
            // size. It will use this method to do that.
            #[allow(dead_code)]
            pub fn new(value: Bytes) -> MDADataSize {
                if value > MIN_MDA_DATA_REGION_SIZE {
                    MDADataSize(value)
                } else {
                    MDADataSize::default()
                }
            }

            pub fn region_size(self) -> MDARegionSize {
                let bytes = self.0 + MDA_REGION_HDR_SIZE;
                let sectors = bytes.sectors();
                MDARegionSize(if sectors.bytes() != bytes {
                    sectors + Sectors(1)
                } else {
                    sectors
                })
            }

            pub fn bytes(self) -> Bytes {
                self.0
            }
        }
    }

    /// Manages the MDA regions which hold the variable length metadata.
    #[derive(Debug)]
    pub struct MDARegions {
        /// The size of a single MDA region. The MDAHeader occupies the
        /// first few bytes of its region, the rest is available for the
        /// variable length metadata.
        region_size: MDARegionSize,
        /// The MDA headers which contain information about the variable
        /// length metadata. NUM_PRIMARY_MDA_REGIONS is 2: in the general
        /// case one is more recently written than the other.
        mdas: [Option<MDAHeader>; mda_size::NUM_PRIMARY_MDA_REGIONS],
    }

    impl MDARegions {
        /// Calculate the offset from start of device for an MDARegion.
        fn mda_offset(header_size: Bytes, index: usize, per_region_size: Bytes) -> u64 {
            *(header_size + per_region_size * index)
        }

        /// The maximum size of variable length metadata that this region
        /// can accommodate.
        pub fn max_data_size(&self) -> MDADataSize {
            self.region_size.data_size()
        }

        /// Initialize the space allotted to the MDA regions to 0.
        /// Return an MDARegions object with uninitialized MDAHeader objects.
        pub fn initialize<F>(
            header_size: Bytes,
            mda_size: MDASize,
            f: &mut F,
        ) -> StratisResult<MDARegions>
        where
            F: Seek + SyncAll,
        {
            let hdr_buf = MDAHeader::default().to_buf();

            let region_size = mda_size.region_size();
            let region_size_bytes = region_size.sectors().bytes();
            for region in 0..mda_size::NUM_MDA_REGIONS {
                f.seek(SeekFrom::Start(MDARegions::mda_offset(
                    header_size,
                    region,
                    region_size_bytes,
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
        pub fn load<F>(
            header_size: Bytes,
            mda_size: MDASize,
            f: &mut F,
        ) -> StratisResult<MDARegions>
        where
            F: Read + Seek,
        {
            let region_size = mda_size.region_size();
            let region_size_bytes = region_size.sectors().bytes();

            // Load a single region at the location specified by index.
            // If it appears that no metadata has been written at the location
            // return None. If it appears that there is metadata, but it has
            // been corrupted, return an error.
            let mut load_a_region = |index: usize| -> StratisResult<Option<MDAHeader>> {
                let mut hdr_buf = [0u8; mda_size::_MDA_REGION_HDR_SIZE];
                f.seek(SeekFrom::Start(MDARegions::mda_offset(
                    header_size,
                    index,
                    region_size_bytes,
                )))?;
                f.read_exact(&mut hdr_buf)?;
                Ok(MDAHeader::from_buf(&hdr_buf)?)
            };

            // Get an MDAHeader for the given index.
            // If there is a failure reading the first, fall back on the
            // second. If there is a failure reading both, return an error.
            let mut get_mda = |index: usize| -> StratisResult<Option<MDAHeader>> {
                load_a_region(index)
                    .or_else(|_| load_a_region(index + mda_size::NUM_PRIMARY_MDA_REGIONS))
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

            let used = Bytes(data.len() as u64);
            let max_available = self.max_data_size().bytes();
            if used > max_available {
                let err_msg = format!(
                    "metadata length {} exceeds region available {}",
                    used, max_available
                );
                return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
            };

            let header = MDAHeader {
                last_updated: *time,
                used,
                data_crc: crc32::checksum_castagnoli(data),
            };
            let hdr_buf = header.to_buf();

            // Write data to a region specified by index.
            let region_size = self.region_size.sectors().bytes();
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
            save_region(older_region + mda_size::NUM_PRIMARY_MDA_REGIONS)?;

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
            let region_size = self.region_size.sectors().bytes();

            // Load the metadata region specified by index.
            // It is an error if the metadata can not be found.
            let mut load_region = |index: usize| -> StratisResult<Vec<u8>> {
                let offset = MDARegions::mda_offset(header_size, index, region_size)
                    + mda_size::_MDA_REGION_HDR_SIZE as u64;
                f.seek(SeekFrom::Start(offset))?;
                mda.load_region(f)
            };

            // TODO: Figure out if there is an action to take if the
            // first read returns an error.
            load_region(newer_region)
                .or_else(|_| load_region(newer_region + mda_size::NUM_PRIMARY_MDA_REGIONS))
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
        /// Return None if there is no MDAHeader to be read. This is detected if the
        /// timestamp region in the buffer is 0.
        fn from_buf(
            buf: &[u8; mda_size::_MDA_REGION_HDR_SIZE],
        ) -> StratisResult<Option<MDAHeader>> {
            if LittleEndian::read_u32(&buf[..4]) != crc32::checksum_castagnoli(&buf[4..]) {
                return Err(StratisError::Engine(
                    ErrorEnum::Invalid,
                    "MDA region header CRC".into(),
                ));
            }

            // Even though hdr_version is positioned later in struct, check it
            // right after the CRC
            let hdr_version = buf[28];
            if hdr_version != STRAT_REGION_HDR_VERSION {
                return Err(StratisError::Engine(
                    ErrorEnum::Invalid,
                    format!("Unknown region header version: {}", hdr_version),
                ));
            }

            let metadata_version = buf[29];
            if metadata_version != STRAT_METADATA_VERSION {
                return Err(StratisError::Engine(
                    ErrorEnum::Invalid,
                    format!("Unknown metadata version: {}", metadata_version),
                ));
            }

            match LittleEndian::read_u64(&buf[16..24]) {
                0 => Ok(None),
                secs => {
                    let used = Bytes(LittleEndian::read_u64(&buf[8..16]));

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

        fn to_buf(&self) -> [u8; mda_size::_MDA_REGION_HDR_SIZE] {
            // Unsigned casts are always safe, as sec and nsec values are never negative
            assert!(self.last_updated.timestamp() >= 0);

            let mut buf = [0u8; mda_size::_MDA_REGION_HDR_SIZE];

            LittleEndian::write_u32(&mut buf[4..8], self.data_crc);
            LittleEndian::write_u64(&mut buf[8..16], *self.used as u64);
            LittleEndian::write_u64(&mut buf[16..24], self.last_updated.timestamp() as u64);
            LittleEndian::write_u32(&mut buf[24..28], self.last_updated.timestamp_subsec_nanos());
            buf[28] = STRAT_REGION_HDR_VERSION;
            buf[29] = STRAT_METADATA_VERSION;

            let buf_crc = crc32::checksum_castagnoli(&buf[4..mda_size::_MDA_REGION_HDR_SIZE]);
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
            #![allow(clippy::absurd_extreme_comparisons)]
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

    #[cfg(test)]
    mod tests {
        use std::io::Cursor;

        use chrono::Utc;
        use proptest::{
            collection::{self, SizeRange},
            num,
        };

        use super::{super::*, *};

        // 82102984128000 in decimal, approx 17 million years
        const UTC_TIMESTAMP_SECS_BOUND: i64 = 0x777_9beb_9f00;
        const UTC_TIMESTAMP_NSECS_BOUND: u32 = 2_000_000_000u32;

        #[test]
        /// Verify that default MDAHeader is all 0s except for CRC and versions.
        fn test_default_mda_header() {
            let buf = MDAHeader::default().to_buf();

            // First 4 bytes is CRC. Then:
            assert!(buf[4..28].iter().all(|x| *x == 0u8));
            assert_eq!(buf[28], STRAT_REGION_HDR_VERSION);
            assert_eq!(buf[29], STRAT_METADATA_VERSION);
            assert!(buf[30..].iter().all(|x| *x == 0u8));
        }

        #[test]
        /// Verify that loading the MDARegions fails if the regions are all 0s.
        /// Verify that loading MDARegions succeeds if the regions are properly
        /// initialized.
        fn test_reading_mda_regions() {
            let buf_length = *(BDA_STATIC_HDR_SIZE + MDASize::default().sectors().bytes()) as usize;
            let mut buf = Cursor::new(vec![0; buf_length]);
            assert_matches!(
                MDARegions::load(BDA_STATIC_HDR_SIZE, MDASize::default(), &mut buf),
                Err(_)
            );

            MDARegions::initialize(BDA_STATIC_HDR_SIZE, MDASize::default(), &mut buf).unwrap();
            let regions =
                MDARegions::load(BDA_STATIC_HDR_SIZE, MDASize::default(), &mut buf).unwrap();
            assert_matches!(regions.last_update_time(), None);
        }

        proptest! {
            #[test]
            /// Using an arbitrary data buffer, construct an mda header buffer
            /// Read the mda header buffer twice.
            /// Verify that the resulting MDAHeaders have all equal components.
            /// Verify timestamp and data CRC against original values.
            fn mda_header(ref data in collection::vec(num::u8::ANY, SizeRange::default()),
                          // sec < 0: unwritable timestamp
                          // sec == 0: value of 0 is interpreted as no timestamp when read
                          sec in 1..UTC_TIMESTAMP_SECS_BOUND,
                          nsec in 0..UTC_TIMESTAMP_NSECS_BOUND) {

                let header = MDAHeader {
                    last_updated: Utc.timestamp(sec, nsec),
                    used: Bytes(data.len() as u64),
                    data_crc: crc32::checksum_castagnoli(&data),
                };
                let buf = header.to_buf();
                let mda1 = MDAHeader::from_buf(&buf).unwrap().unwrap();
                let mda2 = MDAHeader::from_buf(&buf).unwrap().unwrap();

                prop_assert_eq!(mda1.last_updated, mda2.last_updated);
                prop_assert_eq!(mda1.used, mda2.used);
                prop_assert_eq!(mda1.data_crc, mda2.data_crc);
                prop_assert_eq!(header.last_updated, mda1.last_updated);
                prop_assert_eq!(header.data_crc, mda1.data_crc);
            }
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
            assert_matches!(MDAHeader::from_buf(&buf), Err(_));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use proptest::{
        collection::{vec, SizeRange},
        num, option,
        prelude::BoxedStrategy,
        strategy::Strategy,
    };
    use uuid::Uuid;

    use devicemapper::{Bytes, Sectors, IEC};

    use super::*;

    /// Corrupt a byte at the specified position.
    fn corrupt_byte<F>(f: &mut F, position: u64) -> io::Result<()>
    where
        F: Read + Seek + SyncAll,
    {
        let mut byte_to_corrupt = [0; 1];
        f.seek(SeekFrom::Start(position))?;
        f.read_exact(&mut byte_to_corrupt)?;
        byte_to_corrupt[0] = !byte_to_corrupt[0];
        f.seek(SeekFrom::Start(position))?;
        f.write_all(&byte_to_corrupt)?;
        f.sync_all()?;
        Ok(())
    }

    /// Return a static header with random block device and MDA size.
    /// The block device is less than the minimum, for efficiency in testing.
    fn random_static_header(blkdev_size: u64, mda_size_factor: u32) -> StaticHeader {
        let pool_uuid = Uuid::new_v4();
        let dev_uuid = Uuid::new_v4();
        let mda_size =
            MDADataSize::new(mda::MIN_MDA_DATA_REGION_SIZE + Bytes(u64::from(mda_size_factor * 4)))
                .region_size()
                .mda_size();
        let blkdev_size = (Bytes(IEC::Mi) + Sectors(blkdev_size).bytes()).sectors();
        StaticHeader::new(
            pool_uuid,
            dev_uuid,
            mda_size,
            blkdev_size,
            Utc::now().timestamp() as u64,
        )
    }

    /// Make a static header strategy
    fn static_header_strategy() -> BoxedStrategy<StaticHeader> {
        (0..64u64, 0..64u32)
            .prop_map(|(b, m)| random_static_header(b, m))
            .boxed()
    }

    proptest! {
        #[test]
        /// Construct an arbitrary StaticHeader object.
        /// Verify that the "memory buffer" is unowned.
        /// Initialize a BDA.
        /// Verify that Stratis buffer validates.
        /// Wipe the BDA.
        /// Verify that the buffer is again unowned.
        fn test_ownership(ref sh in static_header_strategy()) {
            let buf_size = *sh.mda_size.sectors().bytes() as usize + _BDA_STATIC_HDR_SIZE;
            let mut buf = Cursor::new(vec![0; buf_size]);
            prop_assert!(StaticHeader::device_identifiers(&mut buf).unwrap().is_none());

            BDA::initialize(
                &mut buf,
                sh.pool_uuid,
                sh.dev_uuid,
                sh.mda_size.region_size().data_size(),
                sh.blkdev_size,
                Utc::now().timestamp() as u64,
            ).unwrap();

            prop_assert!(StaticHeader::device_identifiers(&mut buf)
                         .unwrap()
                         .map(|(t_p, t_d)| t_p == sh.pool_uuid && t_d == sh.dev_uuid)
                         .unwrap_or(false));

            BDA::wipe(&mut buf).unwrap();
            prop_assert!(StaticHeader::device_identifiers(&mut buf).unwrap().is_none());
        }
    }

    proptest! {
        #[test]
        /// Construct an arbitrary StaticHeader object.
        /// Initialize a BDA.
        /// Verify that the last update time is None.
        fn empty_bda(ref sh in static_header_strategy()) {
            let buf_size = *sh.mda_size.sectors().bytes() as usize + _BDA_STATIC_HDR_SIZE;
            let mut buf = Cursor::new(vec![0; buf_size]);
            let bda = BDA::initialize(
                &mut buf,
                sh.pool_uuid,
                sh.dev_uuid,
                sh.mda_size.region_size().data_size(),
                sh.blkdev_size,
                Utc::now().timestamp() as u64,
            ).unwrap();
            prop_assert!(bda.last_update_time().is_none());
        }
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
            sh.mda_size.region_size().data_size(),
            sh.blkdev_size,
            Utc::now().timestamp() as u64,
        )
        .unwrap();

        let timestamp0 = Utc::now();
        let timestamp1 = Utc::now();
        assert_ne!(timestamp0, timestamp1);

        let mut buf = Cursor::new(vec![0; *sh.blkdev_size.bytes() as usize]);
        bda.save_state(&timestamp1, &data, &mut buf).unwrap();

        // Error, because current timestamp is older than written to newer.
        assert_matches!(bda.save_state(&timestamp0, &data, &mut buf), Err(_));

        let timestamp2 = Utc::now();
        let timestamp3 = Utc::now();
        assert_ne!(timestamp2, timestamp3);

        bda.save_state(&timestamp3, &data, &mut buf).unwrap();

        // Error, because current timestamp is older than written to newer.
        assert_matches!(bda.save_state(&timestamp2, &data, &mut buf), Err(_));
    }

    proptest! {
        #[test]
        /// Construct an arbitrary StaticHeader object.
        /// Initialize a BDA.
        /// Save metadata and verify correct update time and state.
        /// Reload BDA and verify that new BDA has correct update time.
        /// Load state using new BDA and verify correct state.
        /// Save metadata again, and reload one more time, verifying new timestamp.
        fn check_state(
            ref sh in static_header_strategy(),
            ref state in vec(num::u8::ANY, SizeRange::default()),
            ref next_state in vec(num::u8::ANY, SizeRange::default())
        ) {
            let buf_size = *sh.mda_size.sectors().bytes() as usize + _BDA_STATIC_HDR_SIZE;
            let mut buf = Cursor::new(vec![0; buf_size]);
            let mut bda = BDA::initialize(
                &mut buf,
                sh.pool_uuid,
                sh.dev_uuid,
                sh.mda_size.region_size().data_size(),
                sh.blkdev_size,
                Utc::now().timestamp() as u64,
            ).unwrap();
            let current_time = Utc::now();
            bda.save_state(&current_time, &state, &mut buf).unwrap();
            let loaded_state = bda.load_state(&mut buf).unwrap();
            prop_assert!(bda.last_update_time().map(|t| t == &current_time).unwrap_or(false));
            prop_assert!(loaded_state.map(|s| &s == state).unwrap_or(false));

            let mut bda = BDA::load(&mut buf).unwrap().unwrap();
            let loaded_state = bda.load_state(&mut buf).unwrap();
            prop_assert!(loaded_state.map(|s| &s == state).unwrap_or(false));
            prop_assert!(bda.last_update_time().map(|t| t == &current_time).unwrap_or(false));

            let current_time = Utc::now();
            bda.save_state(&current_time, &next_state, &mut buf)
                .unwrap();
            let loaded_state = bda.load_state(&mut buf).unwrap();
            prop_assert!(loaded_state.map(|s| &s == next_state).unwrap_or(false));
            prop_assert!(bda.last_update_time().map(|t| t == &current_time).unwrap_or(false));

        }
    }

    proptest! {
        #[test]
        /// Construct an arbitrary StaticHeader object.
        /// Write it to a buffer, read it out and make sure you get the same thing.
        fn static_header(ref sh1 in static_header_strategy()) {
            let buf = sh1.sigblock_to_buf();
            let sh2 = StaticHeader::sigblock_from_buf(&buf).unwrap().unwrap();
            prop_assert_eq!(sh1.pool_uuid, sh2.pool_uuid);
            prop_assert_eq!(sh1.dev_uuid, sh2.dev_uuid);
            prop_assert_eq!(sh1.blkdev_size, sh2.blkdev_size);
            prop_assert_eq!(sh1.mda_size, sh2.mda_size);
            prop_assert_eq!(sh1.reserved_size, sh2.reserved_size);
            prop_assert_eq!(sh1.flags, sh2.flags);
            prop_assert_eq!(sh1.initialization_time, sh2.initialization_time);
        }
    }

    proptest! {
        #[test]
        /// Verify correct reading of the static header if only one of
        /// the two static headers is corrupted. Verify expected behavior
        /// if both are corrupted, which varies depending on whether the
        /// Stratis magic number or some other part of the header is corrupted.
        fn bda_test_recovery(primary in option::of(0..SECTOR_SIZE),
                             secondary in option::of(0..SECTOR_SIZE)) {
            let sh = random_static_header(10000, 4);
            let buf_size = *sh.mda_size.sectors().bytes() as usize + _BDA_STATIC_HDR_SIZE;
            let mut buf = Cursor::new(vec![0; buf_size]);
            BDA::initialize(
                &mut buf,
                sh.pool_uuid,
                sh.dev_uuid,
                sh.mda_size.region_size().data_size(),
                sh.blkdev_size,
                Utc::now().timestamp() as u64,
            ).unwrap();

            let reference_buf = buf.clone();

            if let Some(index) = primary {
                // Corrupt primary copy
                corrupt_byte(&mut buf, (SECTOR_SIZE + index) as u64).unwrap();
            }

            if let Some(index) = secondary {
                // Corrupt secondary copy
                corrupt_byte(&mut buf, (9 * SECTOR_SIZE + index) as u64).unwrap();
            }

            let setup_result = StaticHeader::setup(&mut buf);

            match (primary, secondary) {
                (Some(p_index), Some(s_index)) => {
                    // Setup should fail to find a usable Stratis BDA
                    match (p_index, s_index) {
                        (4..=19, 4..=19) => {
                            // When we corrupt both magics then we believe that
                            // the signature is not ours and will return Ok(None)
                            prop_assert!(setup_result.is_ok() && setup_result.unwrap().is_none());
                        }
                        _ => {
                            prop_assert!(setup_result.is_err());
                        }
                    }

                    // Check buffer, should be different
                    prop_assert_ne!(reference_buf.get_ref(), buf.get_ref());

                }
                _ => {
                    // Setup should work and buffer should be corrected
                    prop_assert!(setup_result.is_ok() && setup_result.unwrap().is_some());

                    // Check buffer, should be corrected.
                    prop_assert_eq!(reference_buf.get_ref(), buf.get_ref());
                }
            }
        }
    }

    #[test]
    /// Test that we re-write the older of two BDAs if they don't match.
    fn bda_test_rewrite_older() {
        let sh = random_static_header(10000, 4);
        let buf_size = *sh.mda_size.sectors().bytes() as usize + _BDA_STATIC_HDR_SIZE;
        let mut buf = Cursor::new(vec![0; buf_size]);
        let ts = Utc::now().timestamp() as u64;

        BDA::initialize(
            &mut buf,
            sh.pool_uuid,
            sh.dev_uuid,
            sh.mda_size.region_size().data_size(),
            sh.blkdev_size,
            ts,
        )
        .unwrap();

        let mut buf_newer = Cursor::new(vec![0; buf_size]);
        BDA::initialize(
            &mut buf_newer,
            sh.pool_uuid,
            sh.dev_uuid,
            sh.mda_size.region_size().data_size(),
            sh.blkdev_size,
            ts + 1,
        )
        .unwrap();

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
            assert_matches!(setup_result, Ok(_));
            assert!(setup_result.unwrap().is_some());

            // We should match the reference buffer
            assert_eq!(reference_buf.get_ref(), buf_newer.get_ref());
        }
    }

}
