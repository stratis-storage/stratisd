// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use crate::engine::{
    strat_engine::{backstore::blockdev::InternalBlockDev, metadata::BDA},
    types::DevUuid,
};

/// Convert a collection of blockdevs to BDAs.
pub fn bds_to_bdas<B>(bds: Vec<B>) -> HashMap<DevUuid, BDA>
where
    B: InternalBlockDev,
{
    bds.into_iter()
        .map(|bd| {
            let bda = bd.into_bda();
            (bda.dev_uuid(), bda)
        })
        .collect()
}

/// Convert datadevs and cachedevs to BDAs on error with the option of adding
/// one additional BDA .
pub fn tiers_to_bdas<B>(
    datadevs: Vec<B>,
    cachedevs: Vec<B>,
    bda: Option<BDA>,
) -> HashMap<DevUuid, BDA>
where
    B: InternalBlockDev,
{
    bds_to_bdas(datadevs)
        .into_iter()
        .chain(bds_to_bdas(cachedevs))
        .chain(bda.map(|bda| (bda.dev_uuid(), bda)))
        .collect::<HashMap<_, _>>()
}
