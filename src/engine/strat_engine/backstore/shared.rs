// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Methods that are shared by the cache tier and the data tier.

use std::collections::{btree_map, BTreeMap, BTreeSet, HashMap};

use devicemapper::{Device, Sectors};

use crate::{
    engine::{
        strat_engine::{
            backstore::blockdevmgr::{BlkDevSegment, Segment},
            serde_structs::BaseDevSave,
        },
        types::DevUuid,
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

#[derive(Debug)]
pub struct Segments {
    // the end, no sectors can be allocated beyond this point
    limit: Sectors,
    // A map of chunks of data allocated for a single blockdev. The LHS
    // is the offset from the start of the device, the RHS is the length.
    // Uses a BTReeMap so that an iteration over the elements in the tree
    // will be ordered by the value of the LHS.
    // Invariant: forall (s, l) in used there is no s_i, l_i and
    // s_(i + 1), l_(i + 1) s.t. s_1 + l_1 >= s_(i+1)
    used: BTreeMap<Sectors, Sectors>,
}

impl Segments {
    /// Create a new Segments struct with the designated limit and no used
    /// ranges.
    pub fn new(limit: Sectors) -> Segments {
        Segments {
            limit,
            used: BTreeMap::new(),
        }
    }

    /// The number of distinct ranges
    pub fn len(&self) -> usize {
        self.used.len()
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
    // If LHS == RHS then they both equal value. If both are None, then
    // used is empty. Returns an error if value > limit.
    // Postcondition: value < limit
    fn locate_prev_and_next(
        &self,
        value: Sectors,
    ) -> StratisResult<(Option<Sectors>, Option<Sectors>)> {
        if value >= self.limit {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!(
                    "value specified for start of range, {}, exceeds limit, {}",
                    value, self.limit
                ),
            ));
        }

        let mut prev = None;
        let mut next = None;
        for (&key, _) in self.used.iter() {
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

        if start + len < start {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!(
                    "range ({}, {}) extends beyond maximum possible size",
                    start, len
                ),
            ));
        }

        if start + len > self.limit {
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
        let mut temp = Segments::new(self.limit);
        for range in ranges.iter() {
            temp.insert(range)?;
        }

        let union = self.union(&temp)?;

        self.used = union.used;

        Ok(())
    }

    /// Take the union of two Segments. Require that both Segments objects have
    /// the same limit, for simplicity.
    pub fn union(&self, other: &Segments) -> StratisResult<Segments> {
        if self.limit != other.limit {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                "limits differ between Segments structs, can not do a union".into(),
            ));
        }

        let keys = self
            .used
            .keys()
            .chain(other.used.keys())
            .cloned()
            .collect::<BTreeSet<Sectors>>();

        let mut union = Segments::new(self.limit);

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

    /// A Segments object that is the complement of self, i.e., its used
    /// ranges are self's free ranges and vice-versa.
    pub fn complement(&self) -> Segments {
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

        Segments {
            limit: self.limit,
            used: free,
        }
    }
}

/// An iterator for Segments
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

/// Given a function that translates a Stratis UUID to a device
/// number, and some metadata that describes a particular segment within
/// a device by means of its Stratis UUID, and its start and offset w/in the
/// device, return the corresponding BlkDevSegment structure.
// This method necessarily takes a reference to a Box because it receives
// the value from a function, and there is no other way for the function to
// be returned from a closure.
// In future, it may be possible to address this better with FnBox.
pub fn metadata_to_segment(
    uuid_to_devno: &HashMap<DevUuid, Device>,
    base_dev_save: &BaseDevSave,
) -> StratisResult<BlkDevSegment> {
    let parent = base_dev_save.parent;
    uuid_to_devno
        .get(&parent)
        .ok_or_else(|| {
            StratisError::Engine(
                ErrorEnum::NotFound,
                format!(
                    "No block device corresponding to stratisd UUID {:?} found",
                    &parent
                ),
            )
        })
        .map(|device| {
            BlkDevSegment::new(
                parent,
                Segment::new(*device, base_dev_save.start, base_dev_save.length),
            )
        })
}

/// Append the second list of BlkDevSegments to the first, or if the last
/// segment of the first argument is adjacent to the first segment of the
/// second argument, merge those two together.
/// Postcondition: left.len() + right.len() - 1 <= result.len()
/// Postcondition: result.len() <= left.len() + right.len()
// FIXME: There is a method that duplicates this algorithm called
// coalesce_segs. These methods should either be unified into a single method
// OR one should go away entirely in solution to:
// https://github.com/stratis-storage/stratisd/issues/762.
pub fn coalesce_blkdevsegs(left: &[BlkDevSegment], right: &[BlkDevSegment]) -> Vec<BlkDevSegment> {
    if left.is_empty() {
        return right.to_vec();
    }
    if right.is_empty() {
        return left.to_vec();
    }

    let mut segments = Vec::with_capacity(left.len() + right.len());
    segments.extend_from_slice(left);

    // Last existing and first new may be contiguous.
    let coalesced = {
        let right_first = right.first().expect("!right.is_empty()");
        let left_last = segments.last_mut().expect("!left.is_empty()");
        if left_last.uuid == right_first.uuid
            && (left_last.segment.start + left_last.segment.length == right_first.segment.start)
        {
            left_last.segment.length += right_first.segment.length;
            true
        } else {
            false
        }
    };

    if coalesced {
        segments.extend_from_slice(&right[1..]);
    } else {
        segments.extend_from_slice(right);
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Verify some proper functioning when allocator initialized with ranges.
    fn test_allocator_initialized_with_range() {
        let mut allocator = Segments::new(Sectors(128));

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
    }

    #[test]
    /// Verify that insert() properly coalesces adjacent allocations.
    fn test_allocator_insert_ranges_contig() {
        let mut allocator = Segments::new(Sectors(128));

        allocator.insert(&(Sectors(20), Sectors(10))).unwrap();
        allocator.insert(&(Sectors(10), Sectors(10))).unwrap();
        allocator.insert(&(Sectors(30), Sectors(10))).unwrap();

        assert_eq!(allocator.used.len(), 1);
        assert_eq!(
            allocator.used.iter().next().unwrap(),
            (&Sectors(10), &Sectors(30))
        );
    }

    #[test]
    /// Verify that the largest possible limit may be used for the
    /// allocator.
    fn test_max_allocator_range() {
        use std::u64::MAX;

        Segments::new(Sectors(MAX));
    }

    #[test]
    /// Verify that if two argument ranges overlap there is an error.
    fn test_allocator_insert_prev_overlap() {
        let mut allocator = Segments::new(Sectors(128));

        let bad_insert_ranges = [(Sectors(21), Sectors(20)), (Sectors(40), Sectors(40))];
        assert_matches!(allocator.insert_all(&bad_insert_ranges), Err(_))
    }

    #[test]
    /// Verify that if two argument ranges overlap there is an error.
    fn test_allocator_insert_next_overlap() {
        let mut allocator = Segments::new(Sectors(128));

        let bad_insert_ranges = [(Sectors(40), Sectors(1)), (Sectors(39), Sectors(2))];
        assert_matches!(allocator.insert_all(&bad_insert_ranges), Err(_))
    }

    #[test]
    /// Verify that insert_ranges() errors when an element outside the range
    /// limit is requested.
    fn test_allocator_failures_overflow_limit() {
        let mut allocator = Segments::new(Sectors(128));

        // overflow limit range
        assert_matches!(allocator.insert(&(Sectors(1), Sectors(128))), Err(_));
    }

    #[test]
    /// Verify that insert_ranges() errors when an element in a requested range
    /// exceeds u64::MAX.
    fn test_allocator_failures_overflow_max() {
        use std::u64::MAX;

        let mut allocator = Segments::new(Sectors(MAX));

        // overflow max u64
        assert_matches!(allocator.insert(&(Sectors(MAX), Sectors(1))), Err(_));
    }
}
