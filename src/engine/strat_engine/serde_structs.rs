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

use std::collections::HashSet;

use serde::Serialize;

use devicemapper::{Sectors, ThinDevId};

use crate::engine::types::{DevUuid, FilesystemUuid};

/// Implements saving struct data to a serializable form. The form should be
/// sufficient, in conjunction with the environment, to reconstruct the
/// saved struct in all its essentials.
pub trait Recordable<T: Serialize> {
    fn record(&self) -> T;
}

// ALL structs that represent variable length metadata in pre-order
// depth-first traversal order. Note that when organized by types rather than
// values the structure is a DAG not a tree. This just means that there are
// some duplicate type definitions which are obviously not defined twice.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PoolSave {
    pub name: String,
    pub backstore: BackstoreSave,
    pub flex_devs: FlexDevsSave,
    pub thinpool_dev: ThinPoolDevSave,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<bool>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackstoreSave {
    pub data_tier: DataTierSave,
    pub cap: CapSave,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_tier: Option<CacheTierSave>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataTierSave {
    pub blockdev: BlockDevSave,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlockDevSave {
    pub allocs: Vec<Vec<BaseDevSave>>,
    pub devs: Vec<BaseBlockDevSave>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BaseDevSave {
    pub parent: DevUuid,
    pub start: Sectors,
    pub length: Sectors,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BaseBlockDevSave {
    pub uuid: DevUuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_info: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_info: Option<String>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapSave {
    pub allocs: Vec<(Sectors, Sectors)>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CacheTierSave {
    pub blockdev: BlockDevSave,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FlexDevsSave {
    pub meta_dev: Vec<(Sectors, Sectors)>,
    pub thin_meta_dev: Vec<(Sectors, Sectors)>,
    pub thin_data_dev: Vec<(Sectors, Sectors)>,
    pub thin_meta_dev_spare: Vec<(Sectors, Sectors)>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ThinPoolDevSave {
    pub data_block_size: Sectors,
    // TODO: This data type should no longer be optional in Stratis 4.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_args: Option<HashSet<String>>,
    // TODO: This data type should no longer be optional in Stratis 4.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fs_limit: Option<u64>,
    // TODO: This data type should no longer be optional in Stratis 4.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_overprov: Option<bool>,
}

// Struct representing filesystem metadata. This metadata is not held in the
// variable length metadata but on a separate filesystem that is maintained
// by stratisd.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilesystemSave {
    pub name: String,
    pub uuid: FilesystemUuid,
    pub thin_id: ThinDevId,
    pub size: Sectors,
    pub created: u64, // Unix timestamp
}
