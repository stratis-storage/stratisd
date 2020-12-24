// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    cmp::min,
    collections::{btree_map, BTreeMap, BTreeSet},
};

use devicemapper::Sectors;

use crate::{
    engine::strat_engine::metadata::BlockdevSize,
    stratis::{ErrorEnum, StratisError, StratisResult},
};

#[derive(Debug)]
/// A structure that keeps a bunch of segments organized by their initial
/// index. This is principally useful for the range allocator.
/// It enforces some invariants:
/// * No 0 length segment
/// * No continguous segments; if two segments would be contiguous they are
/// coalesced into a single segment.
/// * No overlapping segments
/// * No segments that extend beyond limit
pub struct PerDevSegments {
    // the end, no sectors can be allocated beyond this point
    limit: Sectors,
    // A map of chunks of data allocated for a single blockdev. The LHS
    // is the offset from the start of the device, the RHS is the length.
    // Uses a BTReeMap so that an iteration over the elements in the tree
    // will be ordered by the value of the LHS.
    used: BTreeMap<Sectors, Sectors>,
}

impl PerDevSegments {
    /// Create a new PerDevSegments struct with the designated limit and no
    /// used ranges.
    pub fn new(limit: Sectors) -> PerDevSegments {
        PerDevSegments {
            limit,
            used: BTreeMap::new(),
        }
    }

    /// The number of distinct ranges
    pub fn len(&self) -> usize {
        self.used.len()
    }

    /// The number of sectors occupied by all the ranges
    pub fn sum(&self) -> Sectors {
        self.used.values().cloned().sum()
    }

    /// The boundary past which no allocation is considered.
    pub fn limit(&self) -> Sectors {
        self.limit
    }

    pub fn iter(&self) -> Iter {
        Iter {
            items: self.used.iter(),
        }
    }

    // Locate two adjacent keys in used. LHS <= value and RHS >= value.
    // If LHS == RHS then they both equal value.
    // Postcondition: result == (None, None) <=> used.len() == 0
    fn locate_prev_and_next(
        &self,
        value: Sectors,
    ) -> StratisResult<(Option<Sectors>, Option<Sectors>)> {
        let mut prev = None;
        let mut next = None;
        for &key in self.used.keys() {
            if value >= key {
                prev = Some(key);
            }
            if value <= key {
                next = Some(key);
                break;
            }
        }

        Ok((prev, next))
    }

    // Return the result of what should be obtained on an insertion. A None at
    // the start or the end of the tuple indicates a contiguous range, hence
    // a removal from used if it was previously present.
    // Precondition: prev.is_some() -> prev in self.used
    // Precondition: next.is_some() -> next in self.used
    // Precondition: range.1 != 0
    // Postcondition: prev.is_none() -> result.0.is_none()
    // Postcondition: next.is_none() -> result.2.is_none()
    // Postcondition: prev.is_some() && result.0.is_none() -> result.1.0 == prev
    // Postcondition: range.0 + range.1 <= Sectors max
    #[allow(clippy::type_complexity)]
    fn insertion_result(
        &self,
        prev: Option<Sectors>,
        next: Option<Sectors>,
        range: &(Sectors, Sectors),
    ) -> StratisResult<(
        Option<(Sectors, Sectors)>,
        (Sectors, Sectors),
        Option<(Sectors, Sectors)>,
    )> {
        let (start, len) = range;
        let (start, len) = (*start, *len);

        assert!(len != Sectors(0));

        let end = if let Some(end) = start.checked_add(len) {
            end
        } else {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!(
                    "range ({}, {}) extends beyond maximum possible size",
                    start, len
                ),
            ));
        };

        if end > self.limit {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!(
                    "range ({}, {}) extends beyond limit {}",
                    start, len, self.limit
                ),
            ));
        };

        let res: (Sectors, Sectors) = (start, len);

        let (lhs, (new_start, new_len)) = if let Some(prev) = prev {
            let prev_len: Sectors = *self.used.get(&prev).expect("see precondition");
            if prev + prev_len > start {
                return Err(StratisError::Engine(
                    ErrorEnum::Invalid,
                    format!(
                        "range to add ({}, {}) overlaps previous range ({}, {})",
                        start, len, prev, prev_len
                    ),
                ));
            }

            if prev + prev_len == start {
                (None, (prev, prev_len + len))
            } else {
                (Some((prev, prev_len)), res)
            }
        } else {
            (None, res)
        };

        let (res, rhs) = if let Some(next) = next {
            if new_start + new_len > next {
                return Err(StratisError::Engine(
                    ErrorEnum::Invalid,
                    format!(
                        "range to add ({}, {}) overlaps subsequent range starting at {}",
                        new_start, new_len, next
                    ),
                ));
            }

            let next_len: Sectors = *self.used.get(&next).expect("see precondition");

            if new_start + new_len == next {
                ((new_start, new_len + next_len), None)
            } else {
                ((new_start, new_len), Some((next, next_len)))
            }
        } else {
            ((new_start, new_len), None)
        };

        Ok((lhs, res, rhs))
    }

    /// Insert specified range into self. Return an error if there is any
    /// overlap between the specified range and existing ranges. If the range
    /// is contiguous with an existing range, combine the two.
    /// Inserting a 0 length range has no effect.
    pub fn insert(&mut self, range: &(Sectors, Sectors)) -> StratisResult<()> {
        let (start, len) = range;

        if *start > self.limit {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!(
                    "value specified for start of range, {}, exceeds limit, {}",
                    start, self.limit
                ),
            ));
        }

        if *len == Sectors(0) {
            return Ok(());
        }

        let (prev, next) = self.locate_prev_and_next(*start)?;
        let (prev_res, (new_start, new_len), next_res) =
            self.insertion_result(prev, next, range)?;

        if let Some(prev) = prev {
            if prev_res.is_none() {
                self.used.remove(&prev);
            }
        }

        if let Some(next) = next {
            if next_res.is_none() {
                self.used.remove(&next);
            }
        }

        assert!(
            self.used.insert(new_start, new_len).is_none(),
            "removed in previous steps if present"
        );

        Ok(())
    }

    /// Insert specified ranges into self. Return an error if any of the
    /// ranges overlaps with existing ranges in self or if the specified ranges
    /// overlap w/ each other.
    /// The operation is atomic; either all ranges or none will be inserted.
    pub fn insert_all(&mut self, ranges: &[(Sectors, Sectors)]) -> StratisResult<()> {
        let mut temp = PerDevSegments::new(self.limit);
        for range in ranges.iter() {
            temp.insert(range)?;
        }

        let union = self.union(&temp)?;

        self.used = union.used;

        Ok(())
    }

    /// Take the union of two PerDevSegments. Require that both PerDevSegments
    /// objects have the same limit, for simplicity.
    pub fn union(&self, other: &PerDevSegments) -> StratisResult<PerDevSegments> {
        if self.limit != other.limit {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                "limits differ between PerDevSegments structs, can not do a union".into(),
            ));
        }

        let keys = self
            .used
            .keys()
            .chain(other.used.keys())
            .cloned()
            .collect::<BTreeSet<Sectors>>();

        let mut union = PerDevSegments::new(self.limit);

        for key in keys.iter().rev() {
            if let Some(val) = self.used.get(key) {
                union.insert(&(*key, *val))?;
            }
            if let Some(val) = other.used.get(key) {
                union.insert(&(*key, *val))?;
            }
        }

        Ok(union)
    }

    /// A PerDevSegments object that is the complement of self, i.e., its
    /// used ranges are self's free ranges and vice-versa.
    pub fn complement(&self) -> PerDevSegments {
        let mut free = BTreeMap::new();
        let mut prev_end = Sectors(0);
        for (&start, &len) in self.used.iter() {
            let range = start - prev_end;
            if range != Sectors(0) {
                free.insert(prev_end, range);
            }
            prev_end = start + len; // always less than self.limit
        }

        if prev_end < self.limit {
            free.insert(prev_end, self.limit - prev_end);
        }

        PerDevSegments {
            limit: self.limit,
            used: free,
        }
    }

    #[cfg(test)]
    fn invariant(&self) {
        // No segment has 0 len
        assert!(self.used.values().all(|&l| l != Sectors(0)));
        // No adjacent segments are contiguous, no adjacent segments overlap
        assert!(self
            .used
            .iter()
            .collect::<Vec<_>>()
            .windows(2)
            .all(|l| *l[0].0 + *l[0].1 < *l[1].0));
        // Last segment does not extend past limit
        assert!(self
            .used
            .iter()
            .rev()
            .next()
            .map(|(s, l)| *s + *l <= self.limit)
            .unwrap_or(true));
        // The complement really is the complement
        let same = self.complement().complement();
        assert_eq!(same.limit, self.limit);
        assert_eq!(same.used, self.used);
    }
}

/// An iterator for PerDevSegments
pub struct Iter<'a> {
    items: btree_map::Iter<'a, Sectors, Sectors>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a Sectors, &'a Sectors);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.items.next()
    }
}

#[derive(Debug)]
pub struct RangeAllocator {
    segments: PerDevSegments,
}

impl RangeAllocator {
    /// Create a new RangeAllocator with the specified (offset, length)
    /// ranges marked as used.
    pub fn new(
        limit: BlockdevSize,
        initial_used: &[(Sectors, Sectors)],
    ) -> StratisResult<RangeAllocator> {
        let mut segments = PerDevSegments::new(limit.sectors());
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
        self.segments.sum()
    }

    /// Attempt to allocate.
    /// Returns a PerDevSegments object containing the allocated ranges.
    /// If all available sectors are desired, don't use this function.
    /// Write a simple request_all() function to get the result much more
    /// efficiently.
    pub fn request(&mut self, amount: Sectors) -> PerDevSegments {
        let mut segs = PerDevSegments::new(self.segments.limit());
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
        assert_eq!(request.len(), 2);
        assert_eq!(request.sum(), Sectors(28));
        assert_eq!(allocator.used(), Sectors(128));
        assert_eq!(allocator.available(), Sectors(0));

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

    #[test]
    // Verify some proper functioning when allocator initialized with ranges.
    fn test_allocator_initialized_with_range() {
        let mut allocator = PerDevSegments::new(Sectors(128));

        let ranges = [
            (Sectors(20), Sectors(10)),
            (Sectors(10), Sectors(10)),
            (Sectors(30), Sectors(10)),
        ];
        allocator.insert_all(&ranges).unwrap();

        assert_eq!(allocator.used.len(), 1);
        assert_eq!(
            allocator.used.iter().next().unwrap(),
            (&Sectors(10), &Sectors(30))
        );
        allocator.invariant();
    }

    #[test]
    /// Verify that insert() properly coalesces adjacent allocations.
    fn test_allocator_insert_ranges_contig() {
        let mut allocator = PerDevSegments::new(Sectors(128));

        allocator.insert(&(Sectors(20), Sectors(10))).unwrap();
        allocator.insert(&(Sectors(10), Sectors(10))).unwrap();
        allocator.insert(&(Sectors(30), Sectors(10))).unwrap();

        assert_eq!(allocator.used.len(), 1);
        assert_eq!(
            allocator.used.iter().next().unwrap(),
            (&Sectors(10), &Sectors(30))
        );
        allocator.invariant();
    }

    #[test]
    /// Verify that the largest possible limit may be used for the
    /// allocator.
    fn test_max_allocator_range() {
        use std::u64::MAX;

        PerDevSegments::new(Sectors(MAX));
    }

    #[test]
    /// Verify that if two argument ranges overlap there is an error.
    fn test_allocator_insert_prev_overlap() {
        let mut allocator = PerDevSegments::new(Sectors(128));

        let bad_insert_ranges = [(Sectors(21), Sectors(20)), (Sectors(40), Sectors(40))];
        assert_matches!(allocator.insert_all(&bad_insert_ranges), Err(_))
    }

    #[test]
    /// Verify that if two argument ranges overlap there is an error.
    fn test_allocator_insert_next_overlap() {
        let mut allocator = PerDevSegments::new(Sectors(128));

        let bad_insert_ranges = [(Sectors(40), Sectors(1)), (Sectors(39), Sectors(2))];
        assert_matches!(allocator.insert_all(&bad_insert_ranges), Err(_))
    }

    #[test]
    /// Verify that insert() errors when an element outside the range
    /// limit is requested.
    fn test_allocator_failures_overflow_limit() {
        let mut allocator = PerDevSegments::new(Sectors(128));

        // overflow limit range
        assert_matches!(allocator.insert(&(Sectors(1), Sectors(128))), Err(_));
        allocator.invariant();
    }

    #[test]
    /// Verify that insert() errors when an element in a requested range
    /// exceeds u64::MAX.
    fn test_allocator_failures_overflow_max() {
        use std::u64::MAX;

        let mut allocator = PerDevSegments::new(Sectors(MAX));

        // overflow max u64
        assert_matches!(allocator.insert(&(Sectors(MAX), Sectors(1))), Err(_));
        allocator.invariant();
    }

    #[test]
    /// Verify that if used is empty, the values for both prev and next will
    /// be None.
    fn test_allocator_indices_empty() {
        let allocator = PerDevSegments::new(Sectors(400));
        let result = allocator.locate_prev_and_next(Sectors(37)).unwrap();

        assert_eq!(result, (None, None));
        allocator.invariant();
    }

    #[test]
    /// Verify some facts if used is entirely occupied
    fn test_allocator_indices_full() {
        let mut allocator = PerDevSegments::new(Sectors(400));
        allocator.insert(&(Sectors(0), Sectors(400))).unwrap();

        // If index is after 0 only previous will have a value
        let result = allocator.locate_prev_and_next(Sectors(37)).unwrap();
        assert_eq!(result, (Some(Sectors(0)), None));

        // If index is exactly 0, previous and next are both 0.
        let result = allocator.locate_prev_and_next(Sectors(0)).unwrap();
        assert_eq!(result, (Some(Sectors(0)), Some(Sectors(0))));
        allocator.invariant();
    }

    #[test]
    /// Verify that locate_prev_and_next works even if value exceeds limit
    fn test_search_over_limit() {
        let mut allocator = PerDevSegments::new(Sectors(400));
        assert_eq!(
            allocator.locate_prev_and_next(Sectors(500)).unwrap(),
            (None, None)
        );

        allocator.insert(&(Sectors(0), Sectors(400))).unwrap();
        assert_eq!(
            allocator.locate_prev_and_next(Sectors(500)).unwrap(),
            (Some(Sectors(0)), None)
        );

        allocator.invariant();
    }

    #[test]
    /// Verify that a segment of length 0 can not be inserted. Such a segment
    /// is just silently dropped if specified.
    fn test_allocator_zero_length_insertion() {
        let mut allocator = PerDevSegments::new(Sectors(400));
        assert_matches!(allocator.insert(&(Sectors(12), Sectors(0))), Ok(_));
        assert_eq!(allocator.len(), 0);
        allocator.invariant();
    }

    #[test]
    /// Verify invariant on PerDevSegment w/ 0 length
    fn test_allocator_zero_length() {
        let allocator = PerDevSegments::new(Sectors(0));
        allocator.invariant();
    }

    #[test]
    /// Verify that an insertion at the end with 0 length has no effect,
    /// but with 1 length returns an error.
    fn test_allocator_end_behavior() {
        let mut allocator = PerDevSegments::new(Sectors(127));
        allocator.insert(&(Sectors(127), Sectors(0))).unwrap();

        assert_eq!(allocator.len(), 0);

        assert_matches!(allocator.insert(&(Sectors(127), Sectors(1))), Err(_));

        allocator.invariant();
    }
}
