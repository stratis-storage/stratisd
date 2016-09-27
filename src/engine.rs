// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use types::StratisResult;
use std::path::PathBuf;
use pool::StratisPool;


pub trait Engine {
    fn create_pool(&self,
                   name: &str,
                   blockdev_paths: &[PathBuf],
                   raid_level: u16)
                   -> StratisResult<Box<StratisPool>>;

    fn destroy_pool(&self, name: &str) -> StratisResult<()>;
    fn list_pools(&self) -> StratisResult<()>;
}
