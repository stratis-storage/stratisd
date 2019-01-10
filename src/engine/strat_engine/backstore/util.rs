// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::blockdevmgr::BlkDevSegment;

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
