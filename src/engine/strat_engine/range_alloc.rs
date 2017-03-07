// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cmp::min;
use std::u64::MAX;

use devicemapper::Sectors;
use ranged_set::RangedSet;

#[derive(Debug)]
pub struct RangeAllocator {
    limit: Sectors,
    used: RangedSet<u64>,
}

impl RangeAllocator {
    /// Create a new RangeAllocator with all entries in the range unused.
    pub fn new(limit: Sectors) -> RangeAllocator {
        // Implementation of used_ranges means we can't handle limit=MAX
        assert!(*limit < MAX, "limit must be less than 2^64");
        RangeAllocator {
            limit: limit,
            used: RangedSet::new(),
        }
    }

    /// Create a new RangeAllocator with the specified (offset,
    /// length) ranges marked as used.
    pub fn new_with_used(limit: Sectors, initial_used: &[(Sectors, Sectors)]) -> RangeAllocator {
        let mut allocator = RangeAllocator::new(limit);
        allocator.insert_ranges(initial_used);
        allocator
    }

    fn check_for_overflow(&self, off: Sectors, len: Sectors) {
        assert_ne!(off.checked_add(*len), None);
        assert!(off + len <= self.limit, "off+len greater than range limit");
    }

    /// Mark ranges previously marked as unused as now used.
    fn insert_ranges(&mut self, ranges: &[(Sectors, Sectors)]) -> () {
        for &(off, len) in ranges {
            self.check_for_overflow(off, len);

            for val in *off..*off + *len {
                let inserted = self.used.insert(val);
                if inserted == false {
                    panic!(format!("inserted value {} already present", val));
                }
            }
        }
    }

    /// Mark ranges previously marked as used as now unused.
    pub fn remove_ranges(&mut self, to_free: &[(Sectors, Sectors)]) -> () {
        for &(off, len) in to_free {
            self.check_for_overflow(off, len);

            for val in *off..*off + *len {
                let removed = self.used.remove(&val);
                if removed == false {
                    panic!(format!("tried to remove value {} not present in RangedSet", val));
                }
            }
        }
    }

    /// Available sectors
    pub fn available(&self) -> Sectors {
        Sectors((0..*self.limit)
                    .filter(|val| !self.used.contains(val))
                    .count() as u64)
    }

    /// Allocated sectors
    pub fn used(&self) -> Sectors {
        Sectors((0..*self.limit)
                    .filter(|val| self.used.contains(val))
                    .count() as u64)
    }

    /// Get a list of (offset, length) segments that are in use
    fn used_ranges(&self) -> Vec<(Sectors, Sectors)> {
        let mut used = Vec::new();

        // iterate one *past* limit. This ensures a used range extending
        // up to limit will end and be added to used_ranges properly.
        (0..*self.limit + 1).fold(None,
                                  |curr_range, val| match (curr_range, self.used.contains(&val)) {
                                      (None, true) => Some((val, 1)),
                                      (None, false) => None,
                                      (Some((off, len)), true) => Some((off, len + 1)),
                                      (Some((off, len)), false) => {
            used.push((Sectors(off), Sectors(len)));
            None
        }
                                  });

        used
    }

    /// Get a list of (offset, length) segments that are not in use
    fn avail_ranges(&self) -> Vec<(Sectors, Sectors)> {
        let mut free = Vec::new();

        // Insert an entry to mark the end so the fold works correctly
        let mut used = self.used_ranges();
        used.push((self.limit, Sectors(0)));

        used.into_iter()
            .fold(Sectors(0), |prev_end, (start, len)| {
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
            self.insert_ranges(&[used_range]);

            needed = needed - to_use;
        }

        (amount - needed, segs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocator_allocations() {
        let mut allocator = RangeAllocator::new(Sectors(128));

        assert_eq!(allocator.used(), Sectors(0));
        assert_eq!(allocator.available(), Sectors(128));

        allocator.insert_ranges(&[(Sectors(10), Sectors(100))]);

        assert_eq!(allocator.used(), Sectors(100));
        assert_eq!(allocator.available(), Sectors(28));

        let request = allocator.request(Sectors(50));
        assert_eq!(request.0, Sectors(28));
        assert_eq!(allocator.used(), Sectors(128));
        assert_eq!(allocator.available(), Sectors(0));
        assert_eq!(request.1.len(), 2);

        let good_remove_ranges = [(Sectors(21), Sectors(20)), (Sectors(41), Sectors(40))];
        allocator.remove_ranges(&good_remove_ranges);
        assert_eq!(allocator.used(), Sectors(68));
        assert_eq!(allocator.available(), Sectors(60));
    }

    #[test]
    #[should_panic]
    fn test_allocator_failures_alloc_overlap() {
        let mut allocator = RangeAllocator::new(Sectors(128));

        let _request = allocator.request(Sectors(128));

        // overlap
        let bad_remove_ranges = [(Sectors(21), Sectors(20)), (Sectors(40), Sectors(40))];
        allocator.remove_ranges(&bad_remove_ranges);
    }

    #[test]
    #[should_panic]
    fn test_allocator_failures_range_overwrite() {
        let mut allocator = RangeAllocator::new(Sectors(128));

        let _request = allocator.request(Sectors(128));
        assert_eq!(_request.0, Sectors(128));
        assert_eq!(_request.1, &[(Sectors(0), Sectors(128))]);

        // overwriting a used range
        allocator.insert_ranges(&[(Sectors(1), Sectors(1))]);
    }

    #[test]
    #[should_panic]
    fn test_allocator_failures_removing_unused() {
        let mut allocator = RangeAllocator::new(Sectors(128));

        // removing an unused range
        allocator.remove_ranges(&[(Sectors(1), Sectors(1))]);
    }

    #[test]
    #[should_panic]
    fn test_allocator_failures_overflow_limit() {
        let mut allocator = RangeAllocator::new(Sectors(128));

        // overflow limit range
        allocator.insert_ranges(&[(Sectors(1), Sectors(128))]);
    }

    #[test]
    #[should_panic]
    fn test_allocator_failures_overflow_max() {
        use std::u64::MAX;

        let mut allocator = RangeAllocator::new(Sectors(MAX));

        // overflow max u64
        allocator.insert_ranges(&[(Sectors(MAX), Sectors(1))]);
    }
}
