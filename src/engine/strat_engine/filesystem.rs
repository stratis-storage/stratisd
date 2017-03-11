// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
extern crate rand;

use uuid::Uuid;

use devicemapper::DM;

use consts::IEC;

use engine::EngineResult;
use engine::Filesystem;
use engine::strat_engine::thindev::ThinDev;
use engine::strat_engine::thinpooldev::ThinPoolDev;
use super::super::engine::{HasName, HasUuid};
use types::Bytes;


#[derive(Debug)]
pub struct StratFilesystem {
    fs_id: Uuid,
    name: String,
    thin_dev: ThinDev,
}

impl StratFilesystem {
    pub fn new(fs_id: Uuid,
               name: &str,
               dm: &DM,
               thin_pool: &mut ThinPoolDev)
               -> EngineResult<StratFilesystem> {
        // TODO should replace with proper id generation. DM takes a 24 bit
        // number for the thin_id.  Generate a u16 to avoid the possibility of
        // "too big". Should this be moved into the DM binding (or lower)?
        // How can a client be expected to avoid collisions?
        let thin_id = rand::random::<u16>();
        // TODO We don't require a size to be provided for create_filesystems -
        // but devicemapper requires an initial size for a thin provisioned
        // device - currently hard coded to 1GB.
        let mut new_thin_dev = try!(ThinDev::new(name,
                                                 dm,
                                                 thin_pool,
                                                 thin_id as u32,
                                                 Bytes(IEC::Gi).sectors()));
        try!(new_thin_dev.create_fs());
        Ok(StratFilesystem {
            fs_id: fs_id,
            name: name.to_owned(),
            thin_dev: new_thin_dev,
        })
    }
}

impl HasName for StratFilesystem {
    fn name(&self) -> &str {
        &self.name
    }
}

impl HasUuid for StratFilesystem {
    fn uuid(&self) -> &Uuid {
        &self.fs_id
    }
}

impl Filesystem for StratFilesystem {
    fn rename(&mut self, name: &str) {
        self.name = name.to_owned();
    }
}
