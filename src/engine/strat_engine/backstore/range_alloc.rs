// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{cmp::min, collections::BTreeMap};

use devicemapper::Sectors;

use crate::{
    engine::strat_engine::metadata::BlockdevSize,
    stratis::{ErrorEnum, StratisError, StratisResult},
};

#[derive(Debug)]
pub struct RangeAllocator {
    limit: Sectors,
    used: BTreeMap<Sectors, Sectors>,
}

impl RangeAllocator {
    /// Create a new RangeAllocator with the specified (offset, length)
    /// ranges marked as used.
    pub fn new(
        limit: BlockdevSize,
        initial_used: &[(Sectors, Sectors)],
    ) -> StratisResult<RangeAllocator> {
        let mut allocator = RangeAllocator {
            limit: limit.sectors(),
            used: BTreeMap::new(),
        };
        allocator.insert_ranges(initial_used)?;
        Ok(allocator)
    }

    /// The maximum allocation from this manager
    pub fn size(&self) -> BlockdevSize {
        BlockdevSize::new(self.limit)
    }

    fn check_for_overflow(&self, off: Sectors, len: Sectors) -> StratisResult<()> {
        if let Some(sum) = off.checked_add(len) {
            if sum > self.limit {
                let err_msg = format!(
                    "elements in range ({}, {}) exceed limit {}",
                    off, len, self.limit
                );
                return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
            }
        } else {
            let err_msg = format!(
                "elements in range ({}, {}) inexpressible in this format",
                off, len
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
        }
        Ok(())
    }

    /// Mark ranges previously marked as unused as now used.
    /// Return an error if ranges overlap with each other or with previously
    /// inserted ranges.
    /// TODO: Make this operation atomic.
    /// TODO: Consider using a different algorithmic that first sorts ranges
    /// and then merges used and ranges by traversing them in parallel, for
    /// efficiency.
    fn insert_ranges(&mut self, ranges: &[(Sectors, Sectors)]) -> StratisResult<()> {
        for &(off, len) in ranges {
            self.check_for_overflow(off, len)?;

            let prev = self.used.range(..off).rev().next().map(|(k, v)| (*k, *v));

            let mut contig_prev = None;
            if let Some((prev_off, prev_len)) = prev {
                if prev_off + prev_len > off {
                    let err_msg = format!(
                        "range starting at {} overlaps previous range ({}, {})",
                        off, prev_off, prev_len
                    );
                    return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
                }
                if prev_off + prev_len == off {
                    contig_prev = Some((prev_off, prev_len))
                }
            }

            let next = self.used.range(off..).next().map(|(k, v)| (*k, *v));

            let mut contig_next = None;
            if let Some((next_off, next_len)) = next {
                if off + len > next_off {
                    let err_msg = format!(
                        "range ({}, {}) overlaps subsequent range starting at {}",
                        off, len, next_off
                    );
                    return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
                }
                if off + len == next_off {
                    contig_next = Some((next_off, next_len))
                }
            }

            match (contig_prev, contig_next) {
                (None, None) => {
                    self.used.insert(off, len);
                }
                (None, Some((next_off, next_len))) => {
                    // Contig with next, make new entry
                    self.used.insert(off, len + next_len);
                    self.used
                        .remove(&next_off)
                        .expect("matched Some((next_off, ...");
                }
                (Some((prev_off, prev_len)), None) => {
                    // Contig with prev, just extend prev
                    *self
                        .used
                        .get_mut(&prev_off)
                        .expect("matched Some((prev_off, ...") = prev_len + len;
                }
                (Some((prev_off, prev_len)), Some((next_off, next_len))) => {
                    // Contig with both, remove next and extend prev
                    self.used.remove(&next_off);
                    *self
                        .used
                        .get_mut(&prev_off)
                        .expect("matched Some((prev_off, ...") = prev_len + len + next_len;
                }
            }
        }
        Ok(())
    }

    /// Available sectors
    pub fn available(&self) -> Sectors {
        self.limit - self.used()
    }

    /// Allocated sectors
    pub fn used(&self) -> Sectors {
        self.used.values().cloned().sum()
    }

    /// Get a list of (offset, length) segments that are in use
    fn used_ranges(&self) -> Vec<(Sectors, Sectors)> {
        self.used.iter().map(|(k, v)| (*k, *v)).collect()
    }

    /// Get a list of (offset, length) segments that are not in use
    fn avail_ranges(&self) -> Vec<(Sectors, Sectors)> {
        let mut free = Vec::new();

        // Insert an entry to mark the end so the fold works correctly
        let mut used = self.used_ranges();
        used.push((self.limit, Sectors(0)));

        used.into_iter().fold(Sectors(0), |prev_end, (start, len)| {
            if prev_end < start {
                free.push((prev_end, start - prev_end))
            }
            start + len
        });

        free
    }

    /// Attempt to allocate. Returns number of sectors allocated (may
    /// be less than request, including zero) and a Vec<(offset,
    /// length)> of sectors successfully allocated.
    /// If all available sectors are desired, use available() method to
    /// discover that amount.
    pub fn request(&mut self, amount: Sectors) -> (Sectors, Vec<(Sectors, Sectors)>) {
        let mut segs = Vec::new();
        let mut needed = amount;

        for (start, len) in self.avail_ranges() {
            if needed == Sectors(0) {
                break;
            }

            let to_use = min(needed, len);

            let used_range = (start, to_use);
            segs.push(used_range);
            self.insert_ranges(&[used_range])
                .expect("available ranges must be insertable");

            needed -= to_use;
        }

        (amount - needed, segs)
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
        let mut allocator = RangeAllocator::new(BlockdevSize::new(Sectors(128)), &[]).unwrap();

        assert_eq!(allocator.used(), Sectors(0));
        assert_eq!(allocator.available(), Sectors(128));

        allocator
            .insert_ranges(&[(Sectors(10), Sectors(100))])
            .unwrap();

        assert_eq!(allocator.used(), Sectors(100));
        assert_eq!(allocator.available(), Sectors(28));

        let request = allocator.request(Sectors(50));
        assert_eq!(request.0, Sectors(28));
        assert_eq!(allocator.used(), Sectors(128));
        assert_eq!(allocator.available(), Sectors(0));
        assert_eq!(request.1.len(), 2);

        let available = allocator.available();
        allocator.request(available);
        assert_eq!(allocator.available(), Sectors(0));
    }

    #[test]
    // Verify some proper functioning when allocator initialized with ranges.
    fn test_allocator_initialized_with_range() {
        let ranges = [
            (Sectors(20), Sectors(10)),
            (Sectors(10), Sectors(10)),
            (Sectors(30), Sectors(10)),
        ];
        let allocator = RangeAllocator::new(BlockdevSize::new(Sectors(128)), &ranges).unwrap();
        let used = allocator.used_ranges();
        assert_eq!(used.len(), 1);
        assert_eq!(used[0], (Sectors(10), Sectors(30)));
    }

    #[test]
    /// Verify insert_ranges properly coalesces adjacent allocations.
    fn test_allocator_insert_ranges_contig() {
        let mut allocator = RangeAllocator::new(BlockdevSize::new(Sectors(128)), &[]).unwrap();

        allocator
            .insert_ranges(&[(Sectors(20), Sectors(10))])
            .unwrap();
        allocator
            .insert_ranges(&[(Sectors(10), Sectors(10))])
            .unwrap();
        allocator
            .insert_ranges(&[(Sectors(30), Sectors(10))])
            .unwrap();

        let used = allocator.used_ranges();
        assert_eq!(used.len(), 1);
        assert_eq!(used[0], (Sectors(10), Sectors(30)));
    }

    #[test]
    /// Verify that the largest possible limit may be used for the
    /// allocator.
    fn test_max_allocator_range() {
        use std::u64::MAX;

        RangeAllocator::new(BlockdevSize::new(Sectors(MAX)), &[]).unwrap();
    }

    #[test]
    fn test_allocator_insert_prev_overlap() {
        let mut allocator = RangeAllocator::new(BlockdevSize::new(Sectors(128)), &[]).unwrap();

        let bad_insert_ranges = [(Sectors(21), Sectors(20)), (Sectors(40), Sectors(40))];
        assert_matches!(allocator.insert_ranges(&bad_insert_ranges), Err(_))
    }

    #[test]
    fn test_allocator_insert_next_overlap() {
        let mut allocator = RangeAllocator::new(BlockdevSize::new(Sectors(128)), &[]).unwrap();

        let bad_insert_ranges = [(Sectors(40), Sectors(1)), (Sectors(39), Sectors(2))];
        assert_matches!(allocator.insert_ranges(&bad_insert_ranges), Err(_))
    }

    #[test]
    /// Verify that insert_ranges() errors when all sectors have already been
    /// allocated.
    fn test_allocator_failures_range_overwrite() {
        let mut allocator = RangeAllocator::new(BlockdevSize::new(Sectors(128)), &[]).unwrap();

        let request = allocator.request(Sectors(128));
        assert_eq!(request.0, Sectors(128));
        assert_eq!(request.1, &[(Sectors(0), Sectors(128))]);

        assert_matches!(allocator.insert_ranges(&[(Sectors(1), Sectors(1))]), Err(_));
    }

    #[test]
    /// Verify that insert_ranges() errors when an element outside the range
    /// limit is requested.
    fn test_allocator_failures_overflow_limit() {
        let mut allocator = RangeAllocator::new(BlockdevSize::new(Sectors(128)), &[]).unwrap();

        // overflow limit range
        assert_matches!(
            allocator.insert_ranges(&[(Sectors(1), Sectors(128))]),
            Err(_)
        );
    }

    #[test]
    /// Verify that insert_ranges() errors when an element in a requested range
    /// exceeds u64::MAX.
    fn test_allocator_failures_overflow_max() {
        use std::u64::MAX;

        let mut allocator = RangeAllocator::new(BlockdevSize::new(Sectors(MAX)), &[]).unwrap();

        // overflow max u64
        assert_matches!(
            allocator.insert_ranges(&[(Sectors(MAX), Sectors(1))]),
            Err(_)
        );
    }
}
