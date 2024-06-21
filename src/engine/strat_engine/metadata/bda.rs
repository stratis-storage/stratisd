// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io::{Read, Seek};

use chrono::{DateTime, Utc};

use crate::{
    engine::{
        strat_engine::{
            metadata::{
                mda,
                sizes::{BDAExtendedSize, BlockdevSize, MDADataSize, STATIC_HEADER_SIZE},
                static_header::{MetadataLocation, StaticHeader, StratisIdentifiers},
            },
            writing::SyncAll,
        },
        types::{DevUuid, PoolUuid, StratSigblockVersion},
    },
    stratis::StratisResult,
};

#[derive(Debug)]
pub struct BDA {
    pub(in super::super) header: StaticHeader,
    regions: mda::MDARegions,
}

impl Default for BDA {
    fn default() -> BDA {
        BDA::new(
            StratSigblockVersion::V1,
            StratisIdentifiers::new(PoolUuid::nil(), DevUuid::nil()),
            MDADataSize::default(),
            BlockdevSize::default(),
            DateTime::default(),
        )
    }
}

impl BDA {
    pub fn new(
        sigblock_version: StratSigblockVersion,
        identifiers: StratisIdentifiers,
        mda_data_size: MDADataSize,
        blkdev_size: BlockdevSize,
        initialization_time: DateTime<Utc>,
    ) -> BDA {
        let header = StaticHeader::new(
            sigblock_version,
            identifiers,
            mda_data_size,
            blkdev_size,
            initialization_time,
        );

        let regions = mda::MDARegions::new(header.mda_size);

        BDA { header, regions }
    }

    /// Initialize a blockdev with a Stratis BDA.
    pub fn initialize<F>(&self, f: &mut F) -> StratisResult<()>
    where
        F: Seek + SyncAll,
    {
        self.header.write(f, MetadataLocation::Both)?;

        self.regions
            .initialize(STATIC_HEADER_SIZE.sectors().bytes(), f)?;

        Ok(())
    }

    /// Load a BDA on initial setup of a device, given valid StaticHeader.
    /// Returns None if no valid StaticHeader exists.
    /// Returns None if no BDA appears to exist.
    pub fn load<F>(header: StaticHeader, f: &mut F) -> StratisResult<Option<BDA>>
    where
        F: Read + Seek + SyncAll,
    {
        // Assume that, since a valid StaticHeader was found on the device,
        // that this implies that BDA::initialize() was successfully executed
        // sometime in the past. Since that is the case, valid MDA headers
        // were written to the device. Returns an error if there is an error
        // when loading the MDARegions, which can only be caused by an I/O
        // error or invalid MDA headers.
        let regions =
            mda::MDARegions::load(STATIC_HEADER_SIZE.sectors().bytes(), header.mda_size, f)?;

        Ok(Some(BDA { header, regions }))
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
            .save_state(STATIC_HEADER_SIZE.sectors().bytes(), time, metadata, f)
    }

    /// Read latest metadata from the disk
    pub fn load_state<F>(&self, mut f: &mut F) -> StratisResult<Option<Vec<u8>>>
    where
        F: Read + Seek,
    {
        self.regions
            .load_state(STATIC_HEADER_SIZE.sectors().bytes(), &mut f)
    }

    /// The time when the most recent metadata was written to the BDA,
    /// if any.
    pub fn last_update_time(&self) -> Option<&DateTime<Utc>> {
        self.regions.last_update_time()
    }

    /// The UUID of the device.
    pub fn dev_uuid(&self) -> DevUuid {
        self.header.identifiers.device_uuid
    }

    /// The pool and device UUIDs for this device.
    pub fn identifiers(&self) -> StratisIdentifiers {
        self.header.identifiers
    }

    /// The UUID of the device's pool.
    pub fn pool_uuid(&self) -> PoolUuid {
        self.header.identifiers.pool_uuid
    }

    /// The size of the device.
    pub fn dev_size(&self) -> BlockdevSize {
        self.header.blkdev_size
    }

    /// The number of sectors the BDA itself occupies.
    pub fn extended_size(&self) -> BDAExtendedSize {
        self.header.bda_extended_size()
    }

    /// The maximum size of variable length metadata that can be accommodated.
    pub fn max_data_size(&self) -> MDADataSize {
        self.regions.max_data_size()
    }

    /// Timestamp when the device was initialized.
    pub fn initialization_time(&self) -> DateTime<Utc> {
        self.header.initialization_time
    }

    /// Get the sigblock version for this device.
    pub fn sigblock_version(&self) -> StratSigblockVersion {
        self.header.sigblock_version
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, thread, time};

    use proptest::{collection::vec, num};

    use crate::engine::strat_engine::metadata::static_header::tests::{
        random_static_header, static_header_strategy,
    };

    use super::*;

    proptest! {
        #[test]
        /// Construct an arbitrary StaticHeader object.
        /// Initialize a BDA.
        /// Verify that the last update time is None.
        fn empty_bda(ref sh in static_header_strategy()) {
            let buf_size = convert_test!(*sh.mda_size.bda_size().sectors().bytes(), u128, usize);
            let mut buf = Cursor::new(vec![0; buf_size]);

            let bda = BDA::new(
                StratSigblockVersion::V1,
                sh.identifiers,
                sh.mda_size.region_size().data_size(),
                sh.blkdev_size,
                Utc::now(),
            );
            bda.initialize(&mut buf).unwrap();
            let read_results = StaticHeader::read_sigblocks(&mut buf);
            let header = StaticHeader::repair_sigblocks(&mut buf, read_results, StaticHeader::do_nothing).unwrap().unwrap();
            let bda = BDA::load(header, &mut buf).unwrap().unwrap();
            prop_assert!(bda.last_update_time().is_none());
        }
    }

    #[test]
    /// Construct a BDA and verify that an error is returned if timestamp
    /// of saved data is older than timestamp of most recently written data.
    fn test_early_times_err() {
        let data = [0u8; 3];
        let sleep_time = time::Duration::from_secs(1);

        // Construct a BDA.
        let sh = random_static_header(0, 0);
        let mut buf = Cursor::new(vec![
            0;
            convert_test!(
                *sh.blkdev_size.sectors().bytes(),
                u128,
                usize
            )
        ]);
        let mut bda = BDA::new(
            StratSigblockVersion::V1,
            sh.identifiers,
            sh.mda_size.region_size().data_size(),
            sh.blkdev_size,
            Utc::now(),
        );
        bda.initialize(&mut buf).unwrap();

        let timestamp0 = Utc::now();
        thread::sleep(sleep_time);
        let timestamp1 = Utc::now();

        let mut buf = Cursor::new(vec![
            0;
            convert_test!(
                *sh.blkdev_size.sectors().bytes(),
                u128,
                usize
            )
        ]);
        bda.save_state(&timestamp1, &data, &mut buf).unwrap();

        // Error, because current timestamp is older than written to newer.
        assert_matches!(bda.save_state(&timestamp0, &data, &mut buf), Err(_));

        let timestamp2 = Utc::now();
        thread::sleep(sleep_time);
        let timestamp3 = Utc::now();

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
            let buf_size = convert_test!(*sh.mda_size.bda_size().sectors().bytes(), u128, usize);
            let mut buf = Cursor::new(vec![0; buf_size]);
            let mut bda = BDA::new(
                StratSigblockVersion::V1,
                sh.identifiers,
                sh.mda_size.region_size().data_size(),
                sh.blkdev_size,
                Utc::now(),
            );
            bda.initialize(&mut buf).unwrap();
            let current_time = Utc::now();
            bda.save_state(&current_time, state, &mut buf).unwrap();
            let loaded_state = bda.load_state(&mut buf).unwrap();
            prop_assert!(bda.last_update_time().map(|t| t == &current_time).unwrap_or(false));
            prop_assert!(loaded_state.map(|s| &s == state).unwrap_or(false));

            let read_results = StaticHeader::read_sigblocks(&mut buf);
            let header = StaticHeader::repair_sigblocks(&mut buf, read_results, StaticHeader::write_header).unwrap().unwrap();
            let mut bda = BDA::load(header, &mut buf).unwrap().unwrap();
            let loaded_state = bda.load_state(&mut buf).unwrap();
            prop_assert!(loaded_state.map(|s| &s == state).unwrap_or(false));
            prop_assert!(bda.last_update_time().map(|t| t == &current_time).unwrap_or(false));

            let current_time = Utc::now();
            bda.save_state(&current_time, next_state, &mut buf)
                .unwrap();
            let loaded_state = bda.load_state(&mut buf).unwrap();
            prop_assert!(loaded_state.map(|s| &s == next_state).unwrap_or(false));
            prop_assert!(bda.last_update_time().map(|t| t == &current_time).unwrap_or(false));

        }
    }
}
