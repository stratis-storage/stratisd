// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{HashMap, HashSet},
    iter::once,
};

use devicemapper::Sectors;

use crate::engine::strat_engine::backstore::blockdevmgr::BlkDevSegment;

/// This transaction data structure keeps a list of segments associated with block
/// devices, segments from the cap device, and a map associating each cap device
/// segment with one or more block device segments that make it up. Because the
/// request and allocated space are both vectors in the same order, the API
/// for this data structure relies heavily on indices.
pub struct RequestTransaction {
    /// Block device segments
    blockdevmgr: Vec<BlkDevSegment>,
    /// Cap device segments
    backstore: Vec<(Sectors, Sectors)>,
    /// Map between a cap device segment and its corresponding block device segments
    map: HashMap<usize, HashSet<usize>>,
}

impl RequestTransaction {
    /// Add a cap device segment request to be committed later.
    pub fn add_seg_req(&mut self, seg_req: (Sectors, Sectors)) {
        self.backstore.push(seg_req);
    }

    /// Add a block device segment request to be committed later.
    ///
    /// The index must correspond to the appropriate index of the cap device segment
    /// that has been requested. This will permit cancelling part of the request
    /// but not another.
    pub fn add_bd_seg_req(&mut self, seg_req_idx: usize, seg: BlkDevSegment) {
        self.blockdevmgr.push(seg);
        if let Some(is) = self.map.get_mut(&seg_req_idx) {
            is.insert(self.blockdevmgr.len() - 1);
        } else {
            self.map.insert(
                seg_req_idx,
                once(self.blockdevmgr.len() - 1).collect::<HashSet<_>>(),
            );
        }
    }

    /// Drain the block device segments from this transaction data structure and
    /// make them available as an iterator.
    pub fn drain_blockdevmgr(&mut self) -> impl Iterator<Item = BlkDevSegment> + '_ {
        self.blockdevmgr.drain(..)
    }

    /// Get a vector of all block device segments for this transaction.
    pub fn get_blockdevmgr(&self) -> Vec<BlkDevSegment> {
        self.blockdevmgr.clone()
    }

    /// Get a single cap device segment for this transaction by the index associated
    /// with the request.
    pub fn get_backstore_elem(&mut self, idx: usize) -> Option<(Sectors, Sectors)> {
        self.backstore.get(idx).cloned()
    }

    /// Get a list of all cap device segments associated with this transaction.
    pub fn get_backstore(&self) -> Vec<(Sectors, Sectors)> {
        self.backstore.clone()
    }

    /// Drain the cap device segments from this transaction data structure and
    /// make them available as an iterator.
    pub fn drain_backstore(&mut self) -> impl Iterator<Item = (Sectors, Sectors)> + '_ {
        self.backstore.drain(..)
    }

    /// Get all block device segments associated with the cap device request located
    /// at index idx.
    pub fn get_segs_for_req(&self, idx: usize) -> Option<Vec<BlkDevSegment>> {
        self.map.get(&idx).map(|set| {
            self.blockdevmgr
                .iter()
                .cloned()
                .enumerate()
                .filter_map(|(seg_idx, seg)| {
                    if set.contains(&seg_idx) {
                        Some(seg)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
    }

    /// Remove all cap device and block device requests associated with the cap
    /// device request at index idx.
    pub fn remove_request(&mut self, idx: usize) {
        self.backstore.remove(idx);
        let removal_is = self
            .map
            .get(&idx)
            .expect("Cannot have a backstore allocation without blockdev allocations");
        self.blockdevmgr = self
            .blockdevmgr
            .drain(..)
            .enumerate()
            .filter_map(|(idx, seg)| {
                if !removal_is.contains(&idx) {
                    Some(seg)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
    }
}

impl Default for RequestTransaction {
    fn default() -> Self {
        RequestTransaction {
            blockdevmgr: Vec::new(),
            backstore: Vec::new(),
            map: HashMap::new(),
        }
    }
}
