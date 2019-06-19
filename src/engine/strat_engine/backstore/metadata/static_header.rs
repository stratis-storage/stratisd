// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fmt,
    io::{self, Read, Seek, SeekFrom},
    str::from_utf8,
};

use byteorder::{ByteOrder, LittleEndian};
use crc::crc32;
use uuid::Uuid;

use devicemapper::{Bytes, Sectors, SECTOR_SIZE};

use crate::{
    engine::{
        strat_engine::{backstore::metadata::mda::MDASize, device::SyncAll},
        DevUuid, PoolUuid,
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

pub const _BDA_STATIC_HDR_SIZE: usize = 16 * SECTOR_SIZE;
pub const BDA_STATIC_HDR_SIZE: Bytes = Bytes(_BDA_STATIC_HDR_SIZE as u64);

const STRAT_MAGIC: &[u8] = b"!Stra0tis\x86\xff\x02^\x41rh";

const STRAT_SIGBLOCK_VERSION: u8 = 1;

/// Read the bytes corresponding to the two signature blocks in the static
/// header from the device. Return a tuple indicating the success or
/// failure for reading either location.
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
pub fn setup<F>(f: &mut F) -> StratisResult<Option<StaticHeader>>
where
    F: Read + Seek + SyncAll,
{
    match read(f) {
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
                                StaticHeader::write(f, &buf_loc_1, MetadataLocation::Second)?;
                                Ok(Some(loc_1))
                            } else {
                                // The second header block must be newer, so overwrite first
                                // with contents of second.
                                StaticHeader::write(f, &buf_loc_2, MetadataLocation::First)?;
                                Ok(Some(loc_2))
                            }
                        }
                        (None, None) => Ok(None),
                        (Some(loc_1), None) => {
                            // Copy 1 has valid Stratis BDA, copy 2 has no magic, re-write copy 2
                            StaticHeader::write(f, &buf_loc_1, MetadataLocation::Second)?;
                            Ok(Some(loc_1))
                        }
                        (None, Some(loc_2)) => {
                            // Copy 2 has valid Stratis BDA, copy 1 has no magic, re-write copy 1
                            StaticHeader::write(f, &buf_loc_2, MetadataLocation::First)?;
                            Ok(Some(loc_2))
                        }
                    }
                }
                (Ok(loc_1), Err(loc_2)) => {
                    // Re-write copy 2
                    if loc_1.is_some() {
                        StaticHeader::write(f, &buf_loc_1, MetadataLocation::Second)?;
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
                        StaticHeader::write(f, &buf_loc_2, MetadataLocation::First)?;
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
        (Ok(buf_loc_1), Err(_)) => {
            // Copy 1 read OK, 2 resulted in an IO error
            match StaticHeader::sigblock_from_buf(&buf_loc_1) {
                Ok(loc_1) => {
                    if loc_1.is_some() {
                        StaticHeader::write(f, &buf_loc_1, MetadataLocation::Second)?;
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
                        StaticHeader::write(f, &buf_loc_2, MetadataLocation::First)?;
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

/// Write 0s on the entire extent of the static header.
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataLocation {
    Both,
    First,
    Second,
}

#[derive(Eq, PartialEq)]
pub struct StaticHeader {
    pub blkdev_size: Sectors,
    pub pool_uuid: PoolUuid,
    pub dev_uuid: DevUuid,
    pub mda_size: MDASize,
    pub reserved_size: Sectors,
    pub flags: u64,
    /// Seconds portion of DateTime<Utc> value.
    pub initialization_time: u64,
}

impl StaticHeader {
    pub fn new(
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
        mda_size: MDASize,
        reserved_size: Sectors,
        blkdev_size: Sectors,
        initialization_time: u64,
    ) -> StaticHeader {
        StaticHeader {
            blkdev_size,
            pool_uuid,
            dev_uuid,
            mda_size,
            reserved_size,
            flags: 0,
            initialization_time,
        }
    }

    /// Write signature block data according to the value of the which argument.
    /// If first location is specified, write zeroes to empty regions in the
    /// first 8 sectors. If the second location is specified, writes zeroes to
    /// empty regions in the second 8 sectors.
    pub fn write<F>(f: &mut F, sig_buf: &[u8], which: MetadataLocation) -> io::Result<()>
    where
        F: Seek + SyncAll,
    {
        let zeroed = [0u8; 6 * SECTOR_SIZE];
        f.seek(SeekFrom::Start(0))?;

        // Write to a single region in the header. Zeroes the first sector,
        // writes sig_buf to the second sector, and then zeroes the remaining
        // six sectors.
        fn write_region<F>(f: &mut F, sig_buf: &[u8], zeroed: &[u8]) -> io::Result<()>
        where
            F: Seek + SyncAll,
        {
            f.write_all(&zeroed[..SECTOR_SIZE])?; // Zero 1 unused sector
            f.write_all(sig_buf)?;
            f.write_all(&zeroed[..SECTOR_SIZE * 6])?; // Zero 6 unused sectors
            f.sync_all()?;
            Ok(())
        };

        if which == MetadataLocation::Both || which == MetadataLocation::First {
            write_region(f, sig_buf, &zeroed)?;
        } else {
            f.seek(SeekFrom::Start(8 * SECTOR_SIZE as u64))?;
        }

        if which == MetadataLocation::Both || which == MetadataLocation::Second {
            write_region(f, sig_buf, &zeroed)?;
        }
        Ok(())
    }

    /// Generate a buf suitable for writing to blockdev
    pub fn sigblock_to_buf(&self) -> [u8; SECTOR_SIZE] {
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
    pub fn sigblock_from_buf(buf: &[u8]) -> StratisResult<Option<StaticHeader>> {
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
use chrono::Utc;

#[cfg(test)]
use devicemapper::IEC;

#[cfg(test)]
use crate::engine::strat_engine::backstore::metadata::mda::{
    MDADataSize, MIN_MDA_DATA_REGION_SIZE,
};

#[cfg(test)]
/// Return a static header with random block device and MDA size.
/// The block device is less than the minimum, for efficiency in testing.
pub fn random_static_header(
    blkdev_size: u64,
    mda_size_factor: u32,
    reserved_sectors: Sectors,
) -> StaticHeader {
    let pool_uuid = Uuid::new_v4();
    let dev_uuid = Uuid::new_v4();
    let mda_size =
        MDADataSize::new(MIN_MDA_DATA_REGION_SIZE + Bytes(u64::from(mda_size_factor * 4)))
            .region_size()
            .mda_size();
    let blkdev_size = (Bytes(IEC::Mi) + Sectors(blkdev_size).bytes()).sectors();
    StaticHeader::new(
        pool_uuid,
        dev_uuid,
        mda_size,
        reserved_sectors,
        blkdev_size,
        Utc::now().timestamp() as u64,
    )
}

#[cfg(test)]
use proptest::{prelude::BoxedStrategy, strategy::Strategy};

#[cfg(test)]
/// Make a static header strategy
pub fn static_header_strategy() -> BoxedStrategy<StaticHeader> {
    (0..64u64, 0..64u32, 0..64u64)
        .prop_map(|(b, m, r)| random_static_header(b, m, Sectors(r)))
        .boxed()
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, SeekFrom, Write};

    use chrono::Utc;
    use proptest::option;

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
        fn static_header_test_recovery(primary in option::of(0..SECTOR_SIZE),
                             secondary in option::of(0..SECTOR_SIZE)) {
            let sh = random_static_header(10000, 4, Sectors(0));
            let buf_size = *sh.mda_size.sectors().bytes() as usize + _BDA_STATIC_HDR_SIZE;
            let mut buf = Cursor::new(vec![0; buf_size]);

            let sh_now = StaticHeader { initialization_time: Utc::now().timestamp() as u64, ..sh };

            StaticHeader::write(&mut buf, &sh_now.sigblock_to_buf(), MetadataLocation::Both).unwrap();

            let reference_buf = buf.clone();

            if let Some(index) = primary {
                // Corrupt primary copy
                corrupt_byte(&mut buf, (SECTOR_SIZE + index) as u64).unwrap();
            }

            if let Some(index) = secondary {
                // Corrupt secondary copy
                corrupt_byte(&mut buf, (9 * SECTOR_SIZE + index) as u64).unwrap();
            }

            let setup_result = setup(&mut buf);

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
    /// Test that we re-write the older of two signature blocks if they
    /// don't match.
    fn sigblock_test_rewrite_older() {
        let sh = random_static_header(10000, 4, Sectors(0));
        let ts = Utc::now().timestamp() as u64;

        let sh_now = StaticHeader {
            initialization_time: ts,
            ..sh
        };

        let buf_size = *sh.mda_size.sectors().bytes() as usize + _BDA_STATIC_HDR_SIZE;
        let mut buf = Cursor::new(vec![0; buf_size]);
        StaticHeader::write(&mut buf, &sh_now.sigblock_to_buf(), MetadataLocation::Both).unwrap();

        let mut buf_newer = Cursor::new(vec![0; buf_size]);
        let sh_newer = StaticHeader {
            initialization_time: ts + 1,
            ..sh
        };
        StaticHeader::write(
            &mut buf_newer,
            &sh_newer.sigblock_to_buf(),
            MetadataLocation::Both,
        )
        .unwrap();

        // We should always match this reference buffer as it's the newer one.
        let reference_buf = buf_newer.clone();

        for offset in &[SECTOR_SIZE, 9 * SECTOR_SIZE] {
            // Copy the older static header to newer static header buffer
            buf.seek(SeekFrom::Start(*offset as u64)).unwrap();
            buf_newer.seek(SeekFrom::Start(*offset as u64)).unwrap();
            let mut sector = [0u8; SECTOR_SIZE];
            buf.read_exact(&mut sector).unwrap();
            buf_newer.write_all(&sector).unwrap();

            assert_ne!(reference_buf.get_ref(), buf_newer.get_ref());

            let setup_result = setup(&mut buf_newer);
            assert_matches!(setup_result, Ok(_));
            assert!(setup_result.unwrap().is_some());

            // We should match the reference buffer
            assert_eq!(reference_buf.get_ref(), buf_newer.get_ref());
        }
    }

}
