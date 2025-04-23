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

use serde::{Serialize, Serializer};

use devicemapper::{Sectors, ThinDevId};

use crate::engine::types::{DevUuid, Features, FilesystemUuid, ValidatedIntegritySpec};

const MAXIMUM_STRING_SIZE: usize = 255;

// Find the largest index which occurs on a char boundary of value which is no
// greater than len.
// TODO: Replace this method with String::floor_char_boundary when
// possible.
fn our_floor_char_boundary(value: &str, len: usize) -> usize {
    let len = std::cmp::min(len, value.len());

    let mut new_index = len;
    while !value.is_char_boundary(new_index) && new_index != 0 {
        new_index -= 1;
    }

    new_index
}

// Return a new String, split at the highest index which lies on a char
// boundary of value which is no greater than len.
fn safe_split_at(value: &str, len: usize) -> &str {
    value.split_at(our_floor_char_boundary(value, len)).0
}

// Serialize a string. Only the first MAXIMUM_STRING_SIZE bytes of the string
// are serialized.
fn serialize_string<S>(value: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    str::serialize(safe_split_at(value, MAXIMUM_STRING_SIZE), serializer)
}

// Serialize an optional string. Only the first MAXIMUM_STRING_SIZE bytes of
// the string are serialized.
fn serialize_option_string<S>(value: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    Option::serialize(
        &value
            .as_ref()
            .map(|v| safe_split_at(v, MAXIMUM_STRING_SIZE)),
        serializer,
    )
}

/// Implements saving struct data to a serializable form. The form should be
/// sufficient, in conjunction with the environment, to reconstruct the
/// saved struct in all its essentials.
pub trait Recordable<T: Serialize> {
    fn record(&self) -> T;
}

/// List of optional features for pools.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize, Hash, Copy, Clone)]
pub enum PoolFeatures {
    Raid,
    Integrity,
    Encryption,
}

impl From<Vec<PoolFeatures>> for Features {
    fn from(v: Vec<PoolFeatures>) -> Self {
        Features {
            encryption: v.contains(&PoolFeatures::Encryption),
        }
    }
}

// ALL structs that represent variable length metadata in pre-order
// depth-first traversal order. Note that when organized by types rather than
// values the structure is a DAG not a tree. This just means that there are
// some duplicate type definitions which are obviously not defined twice.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PoolSave {
    #[serde(serialize_with = "serialize_string")]
    pub name: String,
    pub backstore: BackstoreSave,
    pub flex_devs: FlexDevsSave,
    pub thinpool_dev: ThinPoolDevSave,
    // TODO: This data type should no longer be optional in Stratis 4.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub features: Vec<PoolFeatures>,
}

impl PoolSave {
    /// Return PoolSave with no cache info and all UUIDs specified in cache
    pub fn decache(self) -> (PoolSave, Vec<DevUuid>) {
        let cache_uuids = self.backstore.cache_uuids();
        (
            PoolSave {
                backstore: self.backstore.decache(),
                ..self
            },
            cache_uuids,
        )
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackstoreSave {
    pub data_tier: DataTierSave,
    pub cap: CapSave,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_tier: Option<CacheTierSave>,
}

impl BackstoreSave {
    // Return new struct with cache data removed
    fn decache(self) -> BackstoreSave {
        BackstoreSave {
            cache_tier: None,
            ..self
        }
    }

    // Return UUIDs of all cache devices
    fn cache_uuids(&self) -> Vec<DevUuid> {
        self.cache_tier
            .as_ref()
            .map(|ct| ct.uuids())
            .unwrap_or_default()
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataTierSave {
    pub blockdev: BlockDevSave,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity_spec: Option<ValidatedIntegritySpec>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlockDevSave {
    pub allocs: Vec<Vec<BaseDevSave>>,
    pub devs: Vec<BaseBlockDevSave>,
}

impl BlockDevSave {
    fn uuids(&self) -> Vec<DevUuid> {
        self.devs.iter().map(|bb| bb.uuid).collect::<Vec<_>>()
    }
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub integrity_meta_allocs: Vec<(Sectors, Sectors)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_option_string")]
    pub user_info: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_option_string")]
    pub hardware_info: Option<String>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapSave {
    pub allocs: Vec<(Sectors, Sectors)>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub crypt_meta_allocs: Vec<(Sectors, Sectors)>,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CacheTierSave {
    pub blockdev: BlockDevSave,
}

impl CacheTierSave {
    fn uuids(&self) -> Vec<DevUuid> {
        self.blockdev.uuids()
    }
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
    pub feature_args: Option<Vec<String>>,
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
    #[serde(serialize_with = "serialize_string")]
    pub name: String,
    pub uuid: FilesystemUuid,
    pub thin_id: ThinDevId,
    pub size: Sectors,
    pub created: u64, // Unix timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fs_size_limit: Option<Sectors>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<FilesystemUuid>,
    #[serde(default)]
    pub merge: bool,
}

#[cfg(test)]
mod tests {
    use super::safe_split_at;
    proptest! {
        #[test]
        /// Check safe splitting on random strings.
        fn test_split(s in "\\PC*", index in 0..10usize) {
            let result = safe_split_at(&s, s.len().saturating_sub(index));
            prop_assert_eq!(s.find(result), Some(0));
            // A UTF8 String character requires at most 4 bytes.
            prop_assert!(s.len() - result.len() <= index + 3);

            prop_assert!((index == 0 || s.is_empty()) || result.len() < s.len());
            prop_assert!(result.len() != s.len() || (index == 0 || s.is_empty()));
            prop_assert!(result.is_char_boundary(result.len()));

        }
    }
}
