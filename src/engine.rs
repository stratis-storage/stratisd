// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::RefCell;
use std::path::PathBuf;
use std::collections::HashMap;
use std::collections::BTreeMap;

use types::StratisResult;
use pool::{Pool, StratisPool};


pub trait Engine {
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[PathBuf],
                   raid_level: u16)
                   -> StratisResult<()>;

    fn destroy_pool(&mut self, name: &str) -> StratisResult<()>;
    fn list_pools(&self) -> StratisResult<BTreeMap<String, Box<StratisPool>>>;
}
