// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::{Device, Sectors};

use crate::{
    engine::{strat_engine::backstore::transaction::RequestTransaction, types::PoolUuid},
    stratis::StratisResult,
};

pub mod v1;
pub mod v2;

pub trait InternalBackstore {
    /// Return the device that this tier is currently using.
    /// This may change, depending on whether the backstore is supporting a cache
    /// or not. There may be no device if no data has yet been allocated from
    /// the backstore.
    fn device(&self) -> Option<Device>;

    /// The current size of allocated space on the blockdevs in the data tier.
    fn datatier_allocated_size(&self) -> Sectors;

    /// The current usable size of all the blockdevs in the data tier.
    fn datatier_usable_size(&self) -> Sectors;

    /// The total number of unallocated usable sectors in the
    /// backstore. Includes both in the cap but unallocated as well as not yet
    /// added to cap.
    fn available_in_backstore(&self) -> Sectors;

    /// Satisfy a request for multiple segments. This request must
    /// always be satisfied exactly, None is returned if this can not
    /// be done.
    ///
    /// Precondition: self.next <= self.size()
    /// Postcondition: self.next <= self.size()
    ///
    /// Postcondition: forall i, sizes_i == result_i.1. The second value
    /// in each pair in the returned vector is therefore redundant, but is
    /// retained as a convenience to the caller.
    /// Postcondition:
    /// forall i, result_i.0 = result_(i - 1).0 + result_(i - 1).1
    fn request_alloc(&mut self, sizes: &[Sectors]) -> StratisResult<Option<RequestTransaction>>;

    /// Commit space requested by request_alloc() to metadata.
    ///
    /// This method commits the newly allocated data segments and then extends the cap device
    /// to be the same size as the allocated data size.
    fn commit_alloc(
        &mut self,
        pool_uuid: PoolUuid,
        transaction: RequestTransaction,
    ) -> StratisResult<()>;
}
