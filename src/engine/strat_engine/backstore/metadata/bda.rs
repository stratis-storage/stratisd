// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io::{Read, Seek};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use devicemapper::{Bytes, Sectors, IEC, SECTOR_SIZE};

use crate::{
    engine::{
        strat_engine::{
            backstore::metadata::{
                mda,
                static_header::{MetadataLocation, StaticHeader},
                MDADataSize,
            },
            device::SyncAll,
        },
        DevUuid, PoolUuid,
    },
    stratis::StratisResult,
};

const _BDA_STATIC_HDR_SIZE: usize = 16 * SECTOR_SIZE;
const BDA_STATIC_HDR_SIZE: Bytes = Bytes(_BDA_STATIC_HDR_SIZE as u64);

const RESERVED_SECTORS: Sectors = Sectors(3 * IEC::Mi / (SECTOR_SIZE as u64)); // = 3 MiB

#[derive(Debug)]
pub struct BDA {
    header: StaticHeader,
    regions: mda::MDARegions,
}

impl BDA {
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
            RESERVED_SECTORS,
            blkdev_size,
            initialization_time,
        );

        StaticHeader::write(f, &header.sigblock_to_buf(), MetadataLocation::Both)?;

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
        StaticHeader::wipe(f)
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

    /// Retrieve the device and pool UUIDs from a stratis device.
    pub fn device_identifiers<F>(f: &mut F) -> StratisResult<Option<((PoolUuid, DevUuid))>>
    where
        F: Read + Seek + SyncAll,
    {
        StaticHeader::setup(f).map(|sh| sh.map(|sh| (sh.pool_uuid, sh.dev_uuid)))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, SeekFrom, Write};

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
        let mda_size =
            MDADataSize::new(mda::MIN_MDA_DATA_REGION_SIZE + Bytes(u64::from(mda_size_factor * 4)))
                .region_size()
                .mda_size();
        let blkdev_size = (Bytes(IEC::Mi) + Sectors(blkdev_size).bytes()).sectors();
        StaticHeader::new(
            pool_uuid,
            dev_uuid,
            mda_size,
            RESERVED_SECTORS,
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
