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
use std::path::PathBuf;

use uuid::Uuid;

use devicemapper::Sectors;

use super::super::errors::EngineResult;
use super::super::types::{DevUuid, FilesystemUuid};

/// Implements saving struct data to a serializable form. The form should be
/// sufficient, in conjunction with the environment, to reconstruct the
/// saved struct in all its essentials.
pub trait Recordable<T> {
    fn record(&self) -> EngineResult<T>;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PoolSave {
    pub name: String,
    pub block_devs: HashMap<DevUuid, BlockDevSave>,
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
    pub uuid: FilesystemUuid,
    pub thin_id: u32,
    pub size: Sectors,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FlexDevsSave {
    pub meta_dev: Vec<(Uuid, Sectors, Sectors)>,
    pub thin_meta_dev: Vec<(Uuid, Sectors, Sectors)>,
    pub thin_data_dev: Vec<(Uuid, Sectors, Sectors)>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ThinPoolDevSave {
    pub data_block_size: Sectors,
}
