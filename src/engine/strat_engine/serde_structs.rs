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

use serde::Serialize;

use devicemapper::{Sectors, ThinDevId};

use super::super::types::{DevUuid, FilesystemUuid};

/// Implements saving struct data to a serializable form. The form should be
/// sufficient, in conjunction with the environment, to reconstruct the
/// saved struct in all its essentials.
pub trait Recordable<T: Serialize> {
    fn record(&self) -> T;
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlockDev {
    pub allocs: Vec<LayeredDev>,
    pub devs: Vec<BlockDevSave>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Cap {}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataTierSave {
    pub blockdev: BlockDev,
    pub cap: Cap,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LayeredDev {
    pub parent: DevUuid,
    pub start: Sectors,
    pub length: Sectors,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolSave {
    pub name: String,
    pub backstore: BackstoreSave,
    pub flex_devs: FlexDevsSave,
    pub thinpool_dev: ThinPoolDevSave,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockDevSave {
    pub uuid: DevUuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_info: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_info: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesystemSave {
    pub name: String,
    pub uuid: FilesystemUuid,
    pub thin_id: ThinDevId,
    pub size: Sectors,
    pub created: u64, // Unix timestamp
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlexDevsSave {
    pub meta_dev: Vec<(Sectors, Sectors)>,
    pub thin_meta_dev: Vec<(Sectors, Sectors)>,
    pub thin_data_dev: Vec<(Sectors, Sectors)>,
    pub thin_meta_dev_spare: Vec<(Sectors, Sectors)>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackstoreSave {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_devs: Option<Vec<BlockDevSave>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_segments: Option<Vec<LayeredDev>>,
    pub data_tier: DataTierSave,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta_segments: Option<Vec<LayeredDev>>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinPoolDevSave {
    pub data_block_size: Sectors,
}
