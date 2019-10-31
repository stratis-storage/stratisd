// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub use self::{
    bda_size::{BDAExtendedSize, ReservedSize},
    blkdev_size::BlockdevSize,
    mda_size::{MDADataSize, MDARegionSize, MDASize},
    static_header_size::{StaticHeaderSize, STATIC_HEADER_SIZE},
};

/// A module which defines constants and types related to static header sizes.
pub mod static_header_size {

    use devicemapper::Sectors;

    pub const PRE_SIGBLOCK_PADDING_SECTORS: usize = 1;
    pub const SIGBLOCK_SECTORS: usize = 1;
    pub const POST_SIGBLOCK_PADDING_SECTORS: usize = 6;
    pub const SIGBLOCK_REGION_SECTORS: usize =
        PRE_SIGBLOCK_PADDING_SECTORS + SIGBLOCK_SECTORS + POST_SIGBLOCK_PADDING_SECTORS;
    pub const STATIC_HEADER_SECTORS: usize = 2 * SIGBLOCK_REGION_SECTORS;

    pub const FIRST_SIGBLOCK_START_SECTORS: usize = PRE_SIGBLOCK_PADDING_SECTORS;
    pub const SECOND_SIGBLOCK_START_SECTORS: usize =
        SIGBLOCK_REGION_SECTORS + PRE_SIGBLOCK_PADDING_SECTORS;

    /// Type for the unique static header size which is a constant.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct StaticHeaderSize(Sectors);

    pub const STATIC_HEADER_SIZE: StaticHeaderSize =
        StaticHeaderSize(Sectors(STATIC_HEADER_SECTORS as u64));

    impl StaticHeaderSize {
        pub fn sectors(self) -> Sectors {
            self.0
        }
    }
}

/// A module which defines types for three different regions of the MDA:
/// * MDADataSize: the size of the region for variable length metadata
/// * MDARegionSize: the size a single MDA region
/// * MDASize: the size of the whole MDA
pub mod mda_size {

    use devicemapper::{Bytes, Sectors};

    pub const _MDA_REGION_HDR_SIZE: usize = 32;
    const MDA_REGION_HDR_SIZE: Bytes = Bytes(_MDA_REGION_HDR_SIZE as u128);

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

        pub fn bda_size(self) -> super::bda_size::BDASize {
            super::bda_size::BDASize::new(
                self.0 + Sectors(super::static_header_size::STATIC_HEADER_SECTORS as u64),
            )
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

/// A module which defines constants and types related to BDA sizes.
pub mod bda_size {

    use devicemapper::Sectors;

    /// Defines the size of the whole BDA which does not not include reserved
    /// space.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct BDASize(Sectors);

    impl BDASize {
        pub fn new(value: Sectors) -> BDASize {
            BDASize(value)
        }

        pub fn sectors(self) -> Sectors {
            self.0
        }
    }

    /// The size of the whole BDA + reserved space.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct BDAExtendedSize(Sectors);

    impl BDAExtendedSize {
        pub fn new(value: Sectors) -> BDAExtendedSize {
            BDAExtendedSize(value)
        }

        pub fn sectors(self) -> Sectors {
            self.0
        }
    }

    /// The reserved space located immediately after the BDA proper.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct ReservedSize(Sectors);

    impl ReservedSize {
        pub fn new(value: Sectors) -> ReservedSize {
            ReservedSize(value)
        }

        pub fn sectors(self) -> Sectors {
            self.0
        }
    }
}

// A module which defines types identifying sizes relating to individual
// block devices.
pub mod blkdev_size {
    use devicemapper::Sectors;

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    /// The total size of one entire block device belonging to Stratis.
    /// Note that in the presence of encryption, this is the size of the
    /// dm-crypt device, _not_ the size of the underlying block device.
    pub struct BlockdevSize(Sectors);

    impl BlockdevSize {
        pub fn new(value: Sectors) -> BlockdevSize {
            BlockdevSize(value)
        }

        pub fn sectors(self) -> Sectors {
            self.0
        }
    }
}
