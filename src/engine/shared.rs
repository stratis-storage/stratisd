// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{hash_map::RandomState, HashSet},
    path::{Path, PathBuf},
};

use crate::{
    engine::{
        engine::Pool,
        types::{CreateAction, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

/// Called when the name of a requested pool coincides with the name of an
/// existing pool. Returns an error if the specifications of the requested
/// pool differ from the specifications of the existing pool, otherwise
/// returns Ok(CreateAction::Identity).
pub fn create_pool_idempotent_or_err(
    pool: &dyn Pool,
    pool_name: &str,
    blockdev_paths: &[&Path],
) -> StratisResult<CreateAction<PoolUuid>> {
    let input_devices: HashSet<PathBuf, RandomState> =
        blockdev_paths.iter().map(|p| p.to_path_buf()).collect();

    let existing_paths: HashSet<PathBuf, _> = pool
        .blockdevs()
        .iter()
        .map(|(_, bd)| bd.devnode())
        .collect();

    if input_devices == existing_paths {
        Ok(CreateAction::Identity)
    } else {
        let in_input = input_devices
            .difference(&existing_paths)
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        let in_pool = existing_paths
            .difference(&input_devices)
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        Err(StratisError::Engine(
            ErrorEnum::Invalid,
            create_pool_generate_error_string!(pool_name, in_input, in_pool),
        ))
    }
}
