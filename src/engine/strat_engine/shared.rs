// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, path::Path};

use crate::engine::{
    strat_engine::{
        backstore::{CryptActivationHandle, StratBlockDev},
        metadata::BDA,
    },
    types::DevUuid,
};

/// Convert a collection of blockdevs to BDAs.
pub fn bds_to_bdas(bds: Vec<StratBlockDev>) -> HashMap<DevUuid, BDA> {
    bds.into_iter()
        .map(|bd| (bd.bda.dev_uuid(), bd.bda))
        .collect()
}

/// Convert datadevs and cachedevs to BDAs on error with the option of adding
/// one additional BDA .
pub fn tiers_to_bdas(
    datadevs: Vec<StratBlockDev>,
    cachedevs: Vec<StratBlockDev>,
    bda: Option<BDA>,
) -> HashMap<DevUuid, BDA> {
    bds_to_bdas(datadevs)
        .into_iter()
        .chain(bds_to_bdas(cachedevs))
        .chain(bda.map(|bda| (bda.dev_uuid(), bda)))
        .collect::<HashMap<_, _>>()
}

/// Check that the registered key description and Clevis information for these
/// block devices can unlock at least one of the existing block devices registered.
/// Precondition: self.block_devs must have at least one device.
pub fn can_unlock(physical_path: &Path, try_unlock_keyring: bool, try_unlock_clevis: bool) -> bool {
    CryptActivationHandle::can_unlock(physical_path, try_unlock_keyring, try_unlock_clevis)
}
