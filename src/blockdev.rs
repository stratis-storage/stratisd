// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::rc::Rc;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use types::StratisResult;

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDev {
    pub stratisdev_id: String,
    pub id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum BlockMember {
    Present(Rc<RefCell<BlockDev>>),
}

impl BlockMember {
    pub fn present(&self) -> Option<Rc<RefCell<BlockDev>>> {
        match *self {
            BlockMember::Present(ref x) => Some(x.clone()),
        }
    }
}

impl BlockDev {
    pub fn new(blocksdev_id: &str, path: &Path) -> StratisResult<BlockDev> {

        let bd = BlockDev {
            stratisdev_id: blocksdev_id.to_owned(),
            id: Uuid::new_v4().to_simple_string().to_owned(),
            path: path.to_owned(),
        };

        Ok(bd)
    }
}

#[derive(Debug, Clone)]
pub struct BlockDevs(pub BTreeMap<String, BlockMember>);

impl BlockDevs {
    pub fn new(blockdev_paths: &[PathBuf]) -> StratisResult<BlockDevs> {
        let mut block_devs = BlockDevs(BTreeMap::new());

        let stratis_id = Uuid::new_v4().to_simple_string();

        for path in blockdev_paths {

            let result = BlockDev::new(&stratis_id, path);
            let bd = result.unwrap();

            block_devs.0.insert(bd.id.clone(),
                                BlockMember::Present(Rc::new(RefCell::new(bd))));
        }

        Ok(block_devs)
    }
}
