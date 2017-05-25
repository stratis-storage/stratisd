// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Our current on-disk metadata format uses a JSON-based structure. We
// are using serde-rs, which allows us to implement serialization (to
// JSON) and deserialization (from JSON) by deriving Serialize and
// Deserialize traits.  But, our in-memory structs are too complex for
// serde to handle, and in any case it's best to not tie our real
// structs to what will be saved on disk. Therefore we have *Save
// structs. These contain simple, serde-friendly data types, and we
// can convert to or from them when saving our current state, or
// restoring state from saved metadata.

use std::collections::HashMap;
use std::marker::Sized;
use std::path::PathBuf;

use devicemapper::Sectors;

use engine::EngineResult;

/// Implements saving struct data to a serializable form and reconstructing
/// a struct from that form.
/// Assuming the context of the existing devices this must be an isomorphism,
/// i.e., setup(x.to_save()) == x and setup(x).to_save() == x or it's a bug.
pub trait Isomorphism<T> {
    fn to_save(&self) -> EngineResult<T>;
    fn setup(T) -> Self
        where Self: Sized
    {
        unimplemented!()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PoolSave {
    pub name: String,
    pub block_devs: HashMap<String, BlockDevSave>,
    pub flex_devs: FlexDevsSave,
    pub thinpool_dev: ThinPoolDevSave,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BlockDevSave {
    pub devnode: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FilesystemSave {
    pub name: String,
    pub uuid: String,
    pub thin_id: u32,
    pub size: Sectors,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FlexDevsSave {
    pub meta_dev: Vec<(String, Sectors, Sectors)>,
    pub thin_meta_dev: Vec<(String, Sectors, Sectors)>,
    pub thin_data_dev: Vec<(String, Sectors, Sectors)>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ThinPoolDevSave {
    pub data_block_size: u64,
}
