// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    convert::TryFrom,
    fmt::{self, Debug},
    hash::{Hash, Hasher},
};

use serde_json::Value;

use libcryptsetup_rs::SafeMemHandle;

use crate::stratis::{ErrorEnum, StratisError, StratisResult};

/// A handle for memory designed to safely handle Stratis passphrases. It can
/// be coerced to a slice reference for use in read-only operations.
pub struct SizedKeyMemory {
    mem: SafeMemHandle,
    size: usize,
}

impl SizedKeyMemory {
    pub fn new(mem: SafeMemHandle, size: usize) -> SizedKeyMemory {
        SizedKeyMemory { mem, size }
    }
}

impl Debug for SizedKeyMemory {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.as_ref())
    }
}

impl AsRef<[u8]> for SizedKeyMemory {
    fn as_ref(&self) -> &[u8] {
        &self.mem.as_ref()[..self.size]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptionInfo {
    pub key_description: KeyDescription,
    pub clevis_info: Option<(String, Value)>,
}

impl fmt::Display for EncryptionInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let key_desc_str = format!(
            "key description: \"{}\"",
            self.key_description.as_application_str()
        );
        if let Some((pin, config)) = &self.clevis_info {
            write!(
                f,
                "{}, clevis pin: \"{}\", clevis configuration: \"{}\"",
                key_desc_str, pin, config
            )
        } else {
            write!(f, "{}, no Clevis information", key_desc_str)
        }
    }
}

// Implement Hash explicitly because Value does not implement Hash.
// The reason Value does not implement Hash is that some JSON is really big
// and hashing it would be expensive. This JSON probably won't be, but it
// serves no obvious purpose to stringify it and then hash it.
// Necessary Hash Property: \forall x_1, x_2 in EncryptionInfo,
// if x_1 == x_2, then hash(x_1) == hash(x_2) obviously holds.
#[allow(clippy::derive_hash_xor_eq)]
impl Hash for EncryptionInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key_description.hash(state);
        self.clevis_info.as_ref().map(|(pin, _)| pin).hash(state);
    }
}

impl<'a> Into<Value> for &'a EncryptionInfo {
    fn into(self) -> Value {
        let mut json = json!({"key_description": self.key_description.as_application_str()});
        if let Some(ref info) = self.clevis_info {
            let map = json.as_object_mut().expect("Created a JSON object above");
            map.insert("clevis_pin".to_string(), Value::from(info.0.to_owned()));
            map.insert("clevis_config".to_string(), info.1.clone());
        }
        json
    }
}

/// A data type respresenting a key description for the kernel keyring
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct KeyDescription(String);

impl KeyDescription {
    /// Return the application-level key description (the key description with no
    /// Stratis prefix added).
    pub fn as_application_str(&self) -> &str {
        &self.0
    }
}

// Key descriptions with ';'s are disallowed because a key description
// containing a ';' will not be able to be correctly parsed from the kernel's
// describe string, which uses ';'s as field delimiters.
impl TryFrom<String> for KeyDescription {
    type Error = StratisError;

    fn try_from(s: String) -> StratisResult<KeyDescription> {
        if s.contains(';') {
            Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!("Key description {} contains a ';'", s),
            ))
        } else {
            Ok(KeyDescription(s))
        }
    }
}

impl<'a> TryFrom<&'a String> for KeyDescription {
    type Error = StratisError;

    fn try_from(s: &String) -> StratisResult<KeyDescription> {
        KeyDescription::try_from(s.to_owned())
    }
}
