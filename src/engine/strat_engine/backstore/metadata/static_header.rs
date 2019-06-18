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

use devicemapper::{Sectors, SECTOR_SIZE};

use crate::{
    engine::{
        strat_engine::{backstore::metadata::mda::MDASize, device::SyncAll},
        DevUuid, PoolUuid,
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const _BDA_STATIC_HDR_SIZE: usize = 16 * SECTOR_SIZE;

const STRAT_MAGIC: &[u8] = b"!Stra0tis\x86\xff\x02^\x41rh";

const STRAT_SIGBLOCK_VERSION: u8 = 1;

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
        match StaticHeader::read(f) {
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

    /// Retrieve the device and pool UUIDs from a stratis device.
    pub fn device_identifiers<F>(f: &mut F) -> StratisResult<Option<((PoolUuid, DevUuid))>>
    where
        F: Read + Seek + SyncAll,
    {
        StaticHeader::setup(f).map(|sh| sh.map(|sh| (sh.pool_uuid, sh.dev_uuid)))
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
