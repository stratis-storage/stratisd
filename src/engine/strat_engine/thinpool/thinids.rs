// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for handling thin ids.


use devicemapper::ThinDevId;

use super::super::super::errors::EngineResult;


#[derive(Debug)]
/// A pool of thindev ids, all unique.
pub struct ThinDevIdPool {
    next_id: u32,
}

impl ThinDevIdPool {
    /// Make a new pool from a possibly empty Vec of ids.
    /// Does not verify the absence of duplicate ids.
    pub fn new_from_ids(ids: &[ThinDevId]) -> ThinDevIdPool {
        let max_id: Option<u32> = ids.into_iter().map(|x| (*x).into()).max();
        ThinDevIdPool { next_id: max_id.map(|x| x + 1).unwrap_or(0) }
    }

    /// Get a new id for a thindev.
    /// Returns an error if no thindev id can be constructed.
    // TODO: Improve this so that it is guaranteed only to fail if every 24 bit
    // number has been used.
    pub fn new_id(&mut self) -> EngineResult<ThinDevId> {
        let next_id = ThinDevId::new_u64(u64::from(self.next_id))?;
        self.next_id += 1;
        Ok(next_id)
    }
}
