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
use serde_json::Value;
use uuid::Uuid;

use devicemapper::{Sectors, IEC, SECTOR_SIZE};

use crate::{
    engine::{
        strat_engine::{
            backstore::metadata::sizes::{
                static_header_size, BDAExtendedSize, BlockdevSize, MDASize, ReservedSize,
            },
            device::SyncAll,
        },
        types::{DevUuid, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const RESERVED_SECTORS: Sectors = Sectors(3 * IEC::Mi / (SECTOR_SIZE as u64)); // = 3 MiB

const STRAT_MAGIC: &[u8] = b"!Stra0tis\x86\xff\x02^\x41rh";

const STRAT_SIGBLOCK_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataLocation {
    Both,
    First,
    Second,
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct StratisIdentifiers {
    pub pool_uuid: PoolUuid,
    pub device_uuid: DevUuid,
}

impl StratisIdentifiers {
    pub fn new(pool_uuid: PoolUuid, device_uuid: DevUuid) -> StratisIdentifiers {
        StratisIdentifiers {
            pool_uuid,
            device_uuid,
        }
    }
}

impl fmt::Display for StratisIdentifiers {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Stratis pool UUID: {}, Stratis device UUID: {}",
            self.pool_uuid.to_simple_ref(),
            self.device_uuid.to_simple_ref()
        )
    }
}

impl<'a> Into<Value> for &'a StratisIdentifiers {
    fn into(self) -> Value {
        json!({
            "pool_UUID": Value::from(self.pool_uuid.to_simple_ref().to_string()),
            "device_UUID": Value::from(self.device_uuid.to_simple_ref().to_string())
        })
    }
}

/// Get a Stratis pool UUID and device UUID from any device.
/// If there is an error while obtaining these values return the error.
/// If the device does not appear to be a Stratis device, return None.
pub fn device_identifiers<F>(f: &mut F) -> StratisResult<Option<StratisIdentifiers>>
where
    F: Read + Seek + SyncAll,
{
    StaticHeader::setup(f).map(|sh| sh.map(|sh| sh.identifiers))
}

/// Remove Stratis identifying information from device.
pub fn disown_device<F>(f: &mut F) -> StratisResult<()>
where
    F: Seek + SyncAll,
{
    StaticHeader::wipe(f)
}

#[derive(Debug, Eq, PartialEq)]
pub struct StaticHeader {
    pub blkdev_size: BlockdevSize,
    pub identifiers: StratisIdentifiers,
    pub mda_size: MDASize,
    pub reserved_size: ReservedSize,
    pub flags: u64,
    /// Seconds portion of DateTime<Utc> value.
    pub initialization_time: u64,
}

impl StaticHeader {
    pub fn new(
        identifiers: StratisIdentifiers,
        mda_size: MDASize,
        blkdev_size: BlockdevSize,
        initialization_time: u64,
    ) -> StaticHeader {
        StaticHeader {
            blkdev_size,
            identifiers,
            mda_size,
            reserved_size: ReservedSize::new(RESERVED_SECTORS),
            flags: 0,
            initialization_time,
        }
    }

    /// Read the data at both signature block locations.
    ///
    /// Return the data from each location as an array of bytes
    /// or an error if the read fails. The values are returned
    /// in the same order in which they occur on the device.
    ///
    /// Read the contents of each signature block separately,
    /// as this increases the probability that at least one read
    /// will not fail.
    fn read<F>(
        f: &mut F,
    ) -> (
        io::Result<[u8; bytes!(static_header_size::SIGBLOCK_SECTORS)]>,
        io::Result<[u8; bytes!(static_header_size::SIGBLOCK_SECTORS)]>,
    )
    where
        F: Read + Seek,
    {
        let mut buf_loc_1 = [0u8; bytes!(static_header_size::SIGBLOCK_SECTORS)];
        let mut buf_loc_2 = [0u8; bytes!(static_header_size::SIGBLOCK_SECTORS)];

        fn read_sector_at_offset<F>(f: &mut F, offset: usize, mut buf: &mut [u8]) -> io::Result<()>
        where
            F: Read + Seek,
        {
            f.seek(SeekFrom::Start(offset as u64))
                .and_then(|_| f.read_exact(&mut buf))
        }

        (
            read_sector_at_offset(
                f,
                bytes!(static_header_size::FIRST_SIGBLOCK_START_SECTORS),
                &mut buf_loc_1,
            )
            .map(|_| buf_loc_1),
            read_sector_at_offset(
                f,
                bytes!(static_header_size::SECOND_SIGBLOCK_START_SECTORS),
                &mut buf_loc_2,
            )
            .map(|_| buf_loc_2),
        )
    }

    // Writes signature_block according to the value of which.
    // If first location is specified, write zeroes to empty regions in the
    // first 8 sectors. If the second location is specified, writes zeroes to empty
    // regions in the second 8 sectors.
    pub fn write<F>(&self, f: &mut F, which: MetadataLocation) -> io::Result<()>
    where
        F: Seek + SyncAll,
    {
        let signature_block = self.sigblock_to_buf();
        let zeroed = [0u8; bytes!(static_header_size::POST_SIGBLOCK_PADDING_SECTORS)];
        f.seek(SeekFrom::Start(0))?;

        // Write to a static header region in the static header.
        fn write_region<F>(f: &mut F, signature_block: &[u8], zeroed: &[u8]) -> io::Result<()>
        where
            F: Seek + SyncAll,
        {
            f.write_all(&zeroed[..bytes!(static_header_size::PRE_SIGBLOCK_PADDING_SECTORS)])?;
            f.write_all(signature_block)?;
            f.write_all(&zeroed[..bytes!(static_header_size::POST_SIGBLOCK_PADDING_SECTORS)])?;
            f.sync_all()?;
            Ok(())
        };

        if which == MetadataLocation::Both || which == MetadataLocation::First {
            write_region(f, &signature_block, &zeroed)?;
        } else {
            f.seek(SeekFrom::Start(
                bytes!(static_header_size::SIGBLOCK_REGION_SECTORS) as u64,
            ))?;
        }

        if which == MetadataLocation::Both || which == MetadataLocation::Second {
            write_region(f, &signature_block, &zeroed)?;
        }
        Ok(())
    }

    pub fn bda_extended_size(&self) -> BDAExtendedSize {
        BDAExtendedSize::new(self.mda_size.bda_size().sectors() + self.reserved_size.sectors())
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
        // Action taken when one sigblock is interpreted as invalid.
        //
        // If the other sigblock is interpreted as a Stratis header, attempts repair
        // of the invalid sigblock, returning an error if that fails, otherwise returning
        // the valid sigblock.
        //
        // In all other cases, return the error associated with the invalid sigblock.
        fn ok_err_static_header_handling<F>(
            f: &mut F,
            maybe_sh: Option<StaticHeader>,
            sh_error: StratisError,
            repair_location: MetadataLocation,
        ) -> StratisResult<Option<StaticHeader>>
        where
            F: Seek + SyncAll,
        {
            if let Some(sh) = maybe_sh {
                write_header(f, sh, repair_location)
            } else {
                Err(sh_error)
            }
        }

        // Action taken when both sigblock locations are analyzed without encountering an error.
        //
        // If both sigblocks are interpreted as a Stratis headers,
        // compare contents of static headers.
        //
        // If only a single sigblock is interpreted as a Stratis header,
        // overwrite the other sigblock with the contents of the valid
        // Stratis header sigblock.
        //
        // If neither sigblock is a valid Stratis header,
        // return Ok(None)
        fn ok_ok_static_header_handling<F>(
            f: &mut F,
            maybe_sh1: Option<StaticHeader>,
            maybe_sh2: Option<StaticHeader>,
        ) -> StratisResult<Option<StaticHeader>>
        where
            F: Seek + SyncAll,
        {
            match (maybe_sh1, maybe_sh2) {
                (Some(loc_1), Some(loc_2)) => compare_headers(f, loc_1, loc_2),
                (None, None) => Ok(None),
                (Some(loc_1), None) => write_header(f, loc_1, MetadataLocation::Second),
                (None, Some(loc_2)) => write_header(f, loc_2, MetadataLocation::First),
            }
        }

        // Action taken when there was an I/O error reading the other sigblock.
        //
        // * If this sigblock region is interpreted as having no siglblock, it returns None.
        // * If this sigblock region has a valid sigblock, attempts repair of the other
        //   sigblock region with the valid sigblock, returning the valid sigblock
        //   if the repair succeeds, otherwise returning an error.
        // * If this sigblock appears to be invalid, return the error encountered when
        //   reading the sigblock.
        fn copy_ok_err_handling<F>(
            f: &mut F,
            maybe_sh: StratisResult<Option<StaticHeader>>,
            repair_location: MetadataLocation,
        ) -> StratisResult<Option<StaticHeader>>
        where
            F: Seek + SyncAll,
        {
            match maybe_sh {
                Ok(loc) => {
                    if let Some(ref sh) = loc {
                        sh.write(f, repair_location)?;
                    }
                    Ok(loc)
                }
                Err(e) => Err(e),
            }
        }

        // Action taken when both signature blocks are interpreted as valid
        // Stratis headers.
        //
        // If the contents of the signature blocks are equivalent,
        // return valid static header result.
        //
        // If the contents of the signature blocks are not equivalent,
        // overwrite the older block with the contents of the newer one,
        // or return an error if the blocks have the same initialization time.
        fn compare_headers<F>(
            f: &mut F,
            sh_1: StaticHeader,
            sh_2: StaticHeader,
        ) -> StratisResult<Option<StaticHeader>>
        where
            F: Seek + SyncAll,
        {
            if sh_1 == sh_2 {
                Ok(Some(sh_1))
            } else if sh_1.initialization_time == sh_2.initialization_time {
                let err_str = "Appeared to be a Stratis device, but signature blocks disagree.";
                Err(StratisError::Engine(ErrorEnum::Invalid, err_str.into()))
            } else if sh_1.initialization_time > sh_2.initialization_time {
                write_header(f, sh_1, MetadataLocation::Second)
            } else {
                write_header(f, sh_2, MetadataLocation::First)
            }
        }

        fn write_header<F>(
            f: &mut F,
            sh: StaticHeader,
            repair_location: MetadataLocation,
        ) -> StratisResult<Option<StaticHeader>>
        where
            F: Seek + SyncAll,
        {
            sh.write(f, repair_location)?;
            Ok(Some(sh))
        }

        let (maybe_buf_1, maybe_buf_2) = StaticHeader::read(f);
        match (
            maybe_buf_1.map(|buf| StaticHeader::sigblock_from_buf(&buf)),
            maybe_buf_2.map(|buf| StaticHeader::sigblock_from_buf(&buf)),
        ) {
            (Ok(buf_loc_1), Ok(buf_loc_2)) => match (buf_loc_1, buf_loc_2) {
                (Ok(loc_1), Ok(loc_2)) => ok_ok_static_header_handling(f, loc_1, loc_2),
                (Ok(loc_1), Err(loc_2)) => {
                    ok_err_static_header_handling(f, loc_1, loc_2, MetadataLocation::Second)
                }
                (Err(loc_1), Ok(loc_2)) => {
                    ok_err_static_header_handling(f, loc_2, loc_1, MetadataLocation::First)
                }
                (Err(_), Err(_)) => {
                    let err_str = "Appeared to be a Stratis device, but no valid sigblock found";
                    Err(StratisError::Engine(ErrorEnum::Invalid, err_str.into()))
                }
            },
            (Ok(buf_loc_1), Err(_)) => copy_ok_err_handling(f, buf_loc_1, MetadataLocation::Second),
            (Err(_), Ok(buf_loc_2)) => copy_ok_err_handling(f, buf_loc_2, MetadataLocation::First),
            (Err(_), Err(_)) => {
                let err_str = "Unable to read data at sigblock locations.";
                Err(StratisError::Engine(ErrorEnum::Invalid, err_str.into()))
            }
        }
    }

    /// Generate a buf suitable for writing to blockdev
    fn sigblock_to_buf(&self) -> [u8; bytes!(static_header_size::SIGBLOCK_SECTORS)] {
        let mut buf = [0u8; bytes!(static_header_size::SIGBLOCK_SECTORS)];
        buf[4..20].clone_from_slice(STRAT_MAGIC);
        LittleEndian::write_u64(&mut buf[20..28], *self.blkdev_size.sectors());
        buf[28] = STRAT_SIGBLOCK_VERSION;
        buf[32..64].clone_from_slice(
            self.identifiers
                .pool_uuid
                .to_simple_ref()
                .to_string()
                .as_bytes(),
        );
        buf[64..96].clone_from_slice(
            self.identifiers
                .device_uuid
                .to_simple_ref()
                .to_string()
                .as_bytes(),
        );
        LittleEndian::write_u64(&mut buf[96..104], *self.mda_size.sectors());
        LittleEndian::write_u64(&mut buf[104..112], *self.reserved_size.sectors());
        LittleEndian::write_u64(&mut buf[120..128], self.initialization_time);

        let hdr_crc =
            crc32::checksum_castagnoli(&buf[4..bytes!(static_header_size::SIGBLOCK_SECTORS)]);
        LittleEndian::write_u32(&mut buf[..4], hdr_crc);
        buf
    }

    /// Parse a buffer to a StaticHeader.
    /// Return None if no stratis magic number found.
    /// Return an error if stored checksum and calculated checksum do not
    /// match.
    /// Return an error if the version number is not expected.
    fn sigblock_from_buf(buf: &[u8]) -> StratisResult<Option<StaticHeader>> {
        assert_eq!(buf.len(), bytes!(static_header_size::SIGBLOCK_SECTORS));

        if &buf[4..20] != STRAT_MAGIC {
            return Ok(None);
        }

        let crc = crc32::checksum_castagnoli(&buf[4..bytes!(static_header_size::SIGBLOCK_SECTORS)]);
        if crc != LittleEndian::read_u32(&buf[..4]) {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                "header CRC invalid".into(),
            ));
        }

        let blkdev_size = BlockdevSize::new(Sectors(LittleEndian::read_u64(&buf[20..28])));

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
            identifiers: StratisIdentifiers::new(pool_uuid, dev_uuid),
            blkdev_size,
            mda_size,
            reserved_size: ReservedSize::new(Sectors(LittleEndian::read_u64(&buf[104..112]))),
            flags: 0,
            initialization_time: LittleEndian::read_u64(&buf[120..128]),
        }))
    }

    /// Zero out the entire static header region on the designated file.
    pub fn wipe<F>(f: &mut F) -> StratisResult<()>
    where
        F: Seek + SyncAll,
    {
        let zeroed = [0u8; bytes!(static_header_size::STATIC_HEADER_SECTORS)];
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&zeroed)?;
        f.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use std::io::Cursor;

    use proptest::{option, prelude::BoxedStrategy, strategy::Strategy};
    use uuid::Uuid;

    use chrono::Utc;

    use devicemapper::{Bytes, Sectors, IEC};

    use crate::engine::strat_engine::backstore::metadata::sizes::{
        static_header_size, MDADataSize,
    };

    use super::*;

    proptest! {
        #[test]
        /// Construct an arbitrary StaticHeader object.
        /// Verify that the "memory buffer" is unowned.
        /// Initialize a static header.
        /// Verify that Stratis buffer validates.
        /// Wipe the static header.
        /// Verify that the buffer is again unowned.
        fn test_ownership(ref sh in static_header_strategy()) {
            let buf_size = bytes!(static_header_size::STATIC_HEADER_SECTORS);
            let mut buf = Cursor::new(vec![0; buf_size]);
            prop_assert!(StaticHeader::setup(&mut buf).unwrap().is_none());

            sh.write(&mut buf, MetadataLocation::Both).unwrap();

            prop_assert!(StaticHeader::setup(&mut buf)
                         .unwrap()
                         .map(|new_sh| new_sh.identifiers == sh.identifiers)
                         .unwrap_or(false));

            StaticHeader::wipe(&mut buf).unwrap();
            prop_assert!(StaticHeader::setup(&mut buf).unwrap().is_none());
        }
    }

    /// Return a static header with random block device and MDA size.
    /// The block device is less than the minimum, for efficiency in testing.
    pub fn random_static_header(blkdev_size: u64, mda_size_factor: u32) -> StaticHeader {
        let pool_uuid = Uuid::new_v4();
        let dev_uuid = Uuid::new_v4();
        let mda_size = MDADataSize::new(
            MDADataSize::default().bytes() + Bytes(u64::from(mda_size_factor * 4)),
        )
        .region_size()
        .mda_size();
        let blkdev_size = (Bytes(IEC::Mi) + Sectors(blkdev_size).bytes()).sectors();
        StaticHeader::new(
            StratisIdentifiers::new(pool_uuid, dev_uuid),
            mda_size,
            BlockdevSize::new(blkdev_size),
            Utc::now().timestamp() as u64,
        )
    }

    /// Make a static header strategy
    pub fn static_header_strategy() -> BoxedStrategy<StaticHeader> {
        (0..64u64, 0..64u32)
            .prop_map(|(b, m)| random_static_header(b, m))
            .boxed()
    }

    proptest! {
        #[test]
        /// Verify correct reading of the static header if only one of
        /// the two static headers is corrupted. Verify expected behavior
        /// if both are corrupted, which varies depending on whether the
        /// Stratis magic number or some other part of the header is corrupted.
        fn test_corrupted_sigblock_recovery(primary in option::of(0..bytes!(static_header_size::SIGBLOCK_SECTORS)),
                             secondary in option::of(0..bytes!(static_header_size::SIGBLOCK_SECTORS))) {

            // Corrupt a byte at the specified position.
            fn corrupt_byte<F>(f: &mut F, position: u64) -> io::Result<()>
            where
                F: Read + Seek + SyncAll,
            {
                let mut byte_to_corrupt = [0; 1];
                f.seek(SeekFrom::Start(position))
                    .and_then(|_| f.read_exact(&mut byte_to_corrupt))?;

                byte_to_corrupt[0] = !byte_to_corrupt[0];

                f.seek(SeekFrom::Start(position))
                    .and_then(|_| f.write_all(&byte_to_corrupt))
                    .and_then(|_| f.sync_all())
            }

            let sh = random_static_header(10000, 4);
            let buf_size = bytes!(static_header_size::STATIC_HEADER_SECTORS);

            let mut reference_buf = Cursor::new(vec![0; buf_size]);
            sh.write(&mut reference_buf, MetadataLocation::Both).unwrap();

            let mut buf = Cursor::new(vec![0; buf_size]);
            sh.write(&mut buf, MetadataLocation::Both).unwrap();

            if let Some(index) = primary {
                corrupt_byte(
                    &mut buf,
                    (bytes!(static_header_size::FIRST_SIGBLOCK_START_SECTORS) + index) as u64,
                )
                .unwrap();
            }

            if let Some(index) = secondary {
                corrupt_byte(
                    &mut buf,
                    (bytes!(static_header_size::SECOND_SIGBLOCK_START_SECTORS) + index) as u64,
                )
                .unwrap();
            }

            let setup_result = StaticHeader::setup(&mut buf);

            match (primary, secondary) {
                (Some(p_index), Some(s_index)) => {
                    match (p_index, s_index) {
                        // Both magic locations are corrupted, conclusion is
                        // that this is not a Stratis static header.
                        (4..=19, 4..=19) => {
                            prop_assert!(setup_result.is_ok());
                            prop_assert_eq!(setup_result.unwrap(), None);
                        }
                        // Both sigblocks were corrupted, but at least one
                        // was recognized as a Stratis sigblock.
                        _ => {
                            prop_assert!(setup_result.is_err());
                        }
                    }
                    // No healing was attempted.
                    prop_assert_ne!(reference_buf.get_ref(), buf.get_ref());

                }
                // Only one header was corrupted, so the other was healed.
                _ => {
                    prop_assert!(setup_result.is_ok());
                    prop_assert_eq!(setup_result.unwrap(), Some(sh));
                    prop_assert_eq!(reference_buf.get_ref(), buf.get_ref());
                }
            }
        }
    }

    proptest! {
        #[test]
        /// Construct an arbitrary StaticHeader object.
        /// Write it to a buffer, read it out and make sure you get the same thing.
        fn static_header(ref sh1 in static_header_strategy()) {
            let buf = sh1.sigblock_to_buf();
            let sh2 = StaticHeader::sigblock_from_buf(&buf).unwrap().unwrap();
            prop_assert_eq!(sh1.identifiers, sh2.identifiers);
            prop_assert_eq!(sh1.blkdev_size, sh2.blkdev_size);
            prop_assert_eq!(sh1.mda_size, sh2.mda_size);
            prop_assert_eq!(sh1.reserved_size, sh2.reserved_size);
            prop_assert_eq!(sh1.flags, sh2.flags);
            prop_assert_eq!(sh1.initialization_time, sh2.initialization_time);
        }
    }

    #[test]
    /// Test that the newer sigblock is copied to the older sigblock's location
    /// if the sigblock's timestamps don't match but they are otherwise
    /// identical.
    fn test_rewrite_older_sigblock() {
        let sh = random_static_header(10000, 4);

        let ts = Utc::now().timestamp() as u64;
        let sh_older = StaticHeader::new(sh.identifiers, sh.mda_size, sh.blkdev_size, ts);
        let sh_newer = StaticHeader::new(sh.identifiers, sh.mda_size, sh.blkdev_size, ts + 1);
        assert_ne!(sh_older, sh_newer);

        let buf_size = bytes!(static_header_size::STATIC_HEADER_SECTORS);

        let mut reference_buf = Cursor::new(vec![0; buf_size]);
        sh_newer
            .write(&mut reference_buf, MetadataLocation::Both)
            .unwrap();

        // Test that StaticHeader::setup succeeds by writing the older
        // signature block to the specified older location and the newer
        // sigblock to the specified newer location and then calling
        // StaticHeader::setup. StaticHeader::setup should return without
        // error with the newer sigblock. As a side-effect, it should have
        // overwritten the location of the older sigblock with the value of
        // the newer sigblock.
        let test_rewrite = |sh_older: &StaticHeader,
                            sh_newer: &StaticHeader,
                            older_location: MetadataLocation,
                            newer_location: MetadataLocation| {
            let mut buf = Cursor::new(vec![0; buf_size]);
            sh_older.write(&mut buf, older_location).unwrap();
            sh_newer.write(&mut buf, newer_location).unwrap();
            assert_ne!(buf.get_ref(), reference_buf.get_ref());
            assert_eq!(
                StaticHeader::setup(&mut buf).unwrap().as_ref(),
                Some(sh_newer)
            );
            assert_eq!(buf.get_ref(), reference_buf.get_ref());
        };

        test_rewrite(
            &sh_older,
            &sh_newer,
            MetadataLocation::First,
            MetadataLocation::Second,
        );

        test_rewrite(
            &sh_older,
            &sh_newer,
            MetadataLocation::Second,
            MetadataLocation::First,
        );
    }
}
