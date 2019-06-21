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
    engine::{
        strat_engine::{
            backstore::metadata::{
                mda,
                sizes::{static_header_size, MDADataSize, MDASize},
            },
            device::SyncAll,
        },
        DevUuid, PoolUuid,
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const _BDA_STATIC_HDR_SIZE: usize = static_header_size::STATIC_HEADER_SECTORS * SECTOR_SIZE;
const BDA_STATIC_HDR_SIZE: Bytes = Bytes(_BDA_STATIC_HDR_SIZE as u64);

const RESERVED_SECTORS: Sectors = Sectors(3 * IEC::Mi / (SECTOR_SIZE as u64)); // = 3 MiB

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
            read_sector_at_offset(
                f,
                static_header_size::FIRST_SIGBLOCK_START_SECTORS * SECTOR_SIZE,
                &mut buf_loc_1,
            )
            .map(|_| buf_loc_1),
            read_sector_at_offset(
                f,
                static_header_size::SECOND_SIGBLOCK_START_SECTORS * SECTOR_SIZE,
                &mut buf_loc_2,
            )
            .map(|_| buf_loc_2),
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
        let zeroed = [0u8; static_header_size::POST_SIGBLOCK_PADDING_SECTORS * SECTOR_SIZE];
        f.seek(SeekFrom::Start(0))?;

        // Write to a static header region in the static header.
        fn write_region<F>(f: &mut F, bda_buf: &[u8], zeroed: &[u8]) -> io::Result<()>
        where
            F: Seek + SyncAll,
        {
            f.write_all(&zeroed[..static_header_size::PRE_SIGBLOCK_PADDING_SECTORS * SECTOR_SIZE])?;
            f.write_all(bda_buf)?;
            f.write_all(
                &zeroed[..static_header_size::POST_SIGBLOCK_PADDING_SECTORS * SECTOR_SIZE],
            )?;
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
        mda_data_size: MDADataSize,
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

        // Assume that, since a valid StaticHeader was found on the device,
        // that this implies that BDA::initialize() was succesfully executed
        // sometime in the past. Since that is the case, valid MDA headers
        // were written to the device. Returns an error if there is an error
        // when loading the MDARegions, which can only be caused by an I/O
        // error or invalid MDA headers.
        let regions = mda::MDARegions::load(BDA_STATIC_HDR_SIZE, header.mda_size, f)?;

        Ok(Some(BDA { header, regions }))
    }

    /// Zero out the entire static header region on the designated file.
    pub fn wipe<F>(f: &mut F) -> StratisResult<()>
    where
        F: Seek + SyncAll,
    {
        let zeroed = [0u8; _BDA_STATIC_HDR_SIZE];
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
        self.header.mda_size.bda_size().sectors() + self.header.reserved_size
    }

    /// The maximum size of variable length metadata that can be accommodated.
    pub fn max_data_size(&self) -> MDADataSize {
        self.regions.max_data_size()
    }

    /// Timestamp when the device was initialized.
    pub fn initialization_time(&self) -> u64 {
        self.header.initialization_time
    }

    /// Get a Stratis pool UUID and device UUID from any device.
    /// If there is an error while obtaining these values return the error.
    /// If the device does not appear to be a Stratis device, return None.
    pub fn device_identifiers<F>(f: &mut F) -> StratisResult<Option<((PoolUuid, DevUuid))>>
    where
        F: Read + Seek + SyncAll,
    {
        StaticHeader::setup(f).map(|sh| sh.map(|sh| (sh.pool_uuid, sh.dev_uuid)))
    }
}

#[derive(Eq, PartialEq)]
pub struct StaticHeader {
    blkdev_size: Sectors,
    pool_uuid: PoolUuid,
    dev_uuid: DevUuid,
    mda_size: MDASize,
    reserved_size: Sectors,
    flags: u64,
    /// Seconds portion of DateTime<Utc> value.
    initialization_time: u64,
}

impl StaticHeader {
    fn new(
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
        mda_size: MDASize,
        blkdev_size: Sectors,
        initialization_time: u64,
    ) -> StaticHeader {
        StaticHeader {
            blkdev_size,
            pool_uuid,
            dev_uuid,
            mda_size,
            reserved_size: RESERVED_SECTORS,
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

        let mda_size = MDASize(Sectors(LittleEndian::read_u64(&buf[96..104])));

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

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use proptest::{collection::vec, num, option, prelude::BoxedStrategy, strategy::Strategy};
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
        let mda_size = MDADataSize::new(
            MDADataSize::default().bytes() + Bytes(u64::from(mda_size_factor * 4)),
        )
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
            prop_assert!(BDA::device_identifiers(&mut buf).unwrap().is_none());

            BDA::initialize(
                &mut buf,
                sh.pool_uuid,
                sh.dev_uuid,
                sh.mda_size.region_size().data_size(),
                sh.blkdev_size,
                Utc::now().timestamp() as u64,
            ).unwrap();

            prop_assert!(BDA::device_identifiers(&mut buf)
                         .unwrap()
                         .map(|(t_p, t_d)| t_p == sh.pool_uuid && t_d == sh.dev_uuid)
                         .unwrap_or(false));

            BDA::wipe(&mut buf).unwrap();
            prop_assert!(BDA::device_identifiers(&mut buf).unwrap().is_none());
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
            ref state in vec(num::u8::ANY, 1..100),
            ref next_state in vec(num::u8::ANY, 1..100)
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
                corrupt_byte(&mut buf, (static_header_sectors::FIRST_SIGBLOCK_START_SECTORS * SECTOR_SIZE + index) as u64).unwrap();
            }

            if let Some(index) = secondary {
                // Corrupt secondary copy
                corrupt_byte(&mut buf, (static_header_size::SECOND_SIGBLOCK_START_SECTORS * SECTOR_SIZE + index) as u64).unwrap();
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

        for offset in &[
            static_header_size::FIRST_SIGBLOCK_START_SECTORS * SECTOR_SIZE,
            static_header_size::SECOND_SIGBLOCK_START_SECTORS * SECTOR_SIZE,
        ] {
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
