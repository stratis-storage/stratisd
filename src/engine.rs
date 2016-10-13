// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.


use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::Path;

use types::StratisResult;


pub trait Pool: Debug {
    fn add_blockdev(&mut self, path: &str) -> StratisResult<()>;
    fn add_cachedev(&mut self, path: &str) -> StratisResult<()>;
    fn destroy(&mut self) -> StratisResult<()>;
    fn get_name(&mut self) -> String;
    fn copy(&self) -> Box<Pool>;
}

pub trait Engine: Debug {
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   raid_level: u16)
                   -> StratisResult<()>;

    fn destroy_pool(&mut self, name: &str) -> StratisResult<()>;
    fn list_pools(&self) -> StratisResult<BTreeMap<String, Box<Pool>>>;
}
