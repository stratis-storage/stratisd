// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle cleanup after a failed operation.

use super::super::super::super::stratis::{ErrorEnum, StratisError, StratisResult};

use super::blockdev::StratBlockDev;

/// Wipe some blockdevs of their identifying headers.
/// Return an error if any of the blockdevs could not be wiped.
/// If an error occurs while wiping a blockdev, attempt to wipe all remaining.
pub fn wipe_blockdevs(blockdevs: &[StratBlockDev]) -> StratisResult<()> {
    let mut unerased_devnodes = Vec::new();

    for bd in blockdevs {
        let bd_devnode = bd.devnode.to_owned();
        bd.wipe_metadata()
            .unwrap_or_else(|_| unerased_devnodes.push(bd_devnode));
    }

    if unerased_devnodes.is_empty() {
        Ok(())
    } else {
        let err_msg = format!(
            "Failed to wipe already initialized devnodes: {:?}",
            unerased_devnodes
        );
        Err(StratisError::Engine(ErrorEnum::Error, err_msg))
    }
}
