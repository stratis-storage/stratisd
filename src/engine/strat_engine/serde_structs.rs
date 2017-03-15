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

/// Implements saving and restoring from metadata.
pub trait DSerializable<T> {
    fn to_save(&self) -> T;
    fn setup(T) -> Self
        where Self: Sized
    {
        unimplemented!()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDevSave {
    pub devnode: PathBuf,
    pub total_size: Sectors,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StratSave {
    pub name: String,
    pub id: String,
    pub block_devs: HashMap<String, BlockDevSave>,
}
