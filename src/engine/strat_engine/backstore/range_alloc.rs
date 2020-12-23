// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cmp::min;

use devicemapper::Sectors;

use crate::{
    engine::strat_engine::{backstore::shared::Segments, metadata::BlockdevSize},
    stratis::StratisResult,
};

#[derive(Debug)]
pub struct RangeAllocator {
    segments: Segments,
}

impl RangeAllocator {
    /// Create a new RangeAllocator with the specified (offset, length)
    /// ranges marked as used.
    pub fn new(
        limit: BlockdevSize,
        initial_used: &[(Sectors, Sectors)],
    ) -> StratisResult<RangeAllocator> {
        let mut segments = Segments::new(limit.sectors());
        segments.insert_all(initial_used)?;
        Ok(RangeAllocator { segments })
    }

    /// The maximum allocation from this manager
    pub fn size(&self) -> BlockdevSize {
        BlockdevSize::new(self.segments.limit())
    }

    /// Available sectors
    pub fn available(&self) -> Sectors {
        self.segments.limit() - self.used()
    }

    /// Allocated sectors
    pub fn used(&self) -> Sectors {
        self.segments.iter().map(|(_, l)| l).cloned().sum()
    }

    /// Attempt to allocate.
    /// Returns a Segments object containing the allocated ranges.
    /// If all available sectors are desired, use available() method to
    /// discover that amount.
    pub fn request(&mut self, amount: Sectors) -> Segments {
        let mut segs = Segments::new(self.segments.limit());
        let mut needed = amount;

        for (&start, &len) in self.segments.complement().iter() {
            if needed == Sectors(0) {
                break;
            }
            let to_use = min(needed, len);
            let used_range = (start, to_use);
            segs.insert(&used_range)
                .expect("wholly disjoint from other elements in segs");
            needed -= to_use;
        }

        self.segments = self
            .segments
            .union(&segs)
            .expect("all segments verified to be in available ranges");
        segs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Test proper operation of RangeAllocator.
    /// 1. Instantiate a RangeAllocator.
    /// 2. Verify that no sectors are used (all are available).
    /// 3. Insert range (10, 100) into the allocator.
    /// 4. Verify that 100 sectors are taken and 28 remain.
    /// 5. Request 50 sectors from the allocator.
    /// 6. Verify that the maximum available, 28, were returned in two ranges.
    /// 7. Remove two adjacent ranges of total length 60 sectors.
    /// 8. Verify that number of available sectors is 60, used is 68.
    /// 9. Request all available, then verify that nothing is left.
    fn test_allocator_allocations() {
        let mut allocator = RangeAllocator::new(
            BlockdevSize::new(Sectors(128)),
            &[(Sectors(10), Sectors(100))],
        )
        .unwrap();

        assert_eq!(allocator.used(), Sectors(100));
        assert_eq!(allocator.available(), Sectors(28));

        let request = allocator.request(Sectors(50));
        assert_eq!(allocator.used(), Sectors(128));
        assert_eq!(allocator.available(), Sectors(0));
        assert_eq!(request.len(), 2);

        let available = allocator.available();
        allocator.request(available);
        assert_eq!(allocator.available(), Sectors(0));
    }

    #[test]
    /// Verify that insert_ranges() errors when all sectors have already been
    /// allocated.
    fn test_allocator_failures_range_overwrite() {
        let mut allocator = RangeAllocator::new(BlockdevSize::new(Sectors(128)), &[]).unwrap();

        let request = allocator.request(Sectors(128));
        assert_eq!(allocator.used(), Sectors(128));
        assert_eq!(
            request.iter().collect::<Vec<_>>(),
            vec![(&Sectors(0), &Sectors(128))]
        );

        assert!(allocator.segments.complement().iter().next().is_none());

        assert_matches!(
            allocator.segments.insert_all(&[(Sectors(1), Sectors(1))]),
            Err(_)
        );
    }
}
