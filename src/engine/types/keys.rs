// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    convert::TryFrom,
    fmt::{self, Debug},
    hash::{Hash, Hasher},
};

use serde_json::{Map, Value};

use libcryptsetup_rs::SafeMemHandle;

use crate::{
    engine::types::{ClevisInfo, MaybeInconsistent},
    stratis::{StratisError, StratisResult},
};

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

// This is safe to send across threads because the *mut c_void pointer is always
// allocated in the heap by libcryptsetup which is accessible globally to all threads
// in the process.
unsafe impl Send for SizedKeyMemory {}

impl Debug for SizedKeyMemory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.as_ref())
    }
}

impl AsRef<[u8]> for SizedKeyMemory {
    fn as_ref(&self) -> &[u8] {
        &self.mem.as_ref()[..self.size]
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EncryptionInfo {
    KeyDesc(KeyDescription),
    ClevisInfo(ClevisInfo),
    Both(KeyDescription, ClevisInfo),
}

impl EncryptionInfo {
    /// Convert two optional encryption parameters into an EncryptionInfo enum.
    pub fn from_options(options: (Option<KeyDescription>, Option<ClevisInfo>)) -> Option<Self> {
        match options {
            (Some(kd), Some(ci)) => Some(EncryptionInfo::Both(kd, ci)),
            (Some(kd), _) => Some(EncryptionInfo::KeyDesc(kd)),
            (_, Some(ci)) => Some(EncryptionInfo::ClevisInfo(ci)),
            (None, None) => None,
        }
    }

    /// Set the key description.
    pub fn set_key_desc(self, key_desc: KeyDescription) -> Self {
        match self {
            EncryptionInfo::KeyDesc(_) => EncryptionInfo::KeyDesc(key_desc),
            EncryptionInfo::ClevisInfo(ci) => EncryptionInfo::Both(key_desc, ci),
            EncryptionInfo::Both(_, ci) => EncryptionInfo::Both(key_desc, ci),
        }
    }

    /// Unset the key description. If only Clevis info is set, no action is taken.
    /// Precondition: matches!(self, EncryptionInfo::Both(_, _) | EncryptonInfo::ClevisInfo(_))
    pub fn unset_key_desc(self) -> Self {
        match self {
            EncryptionInfo::KeyDesc(_) => {
                panic!("Cannot convert from encrypted to unencrypted device");
            }
            EncryptionInfo::Both(_, ci) => EncryptionInfo::ClevisInfo(ci),
            unchanged => unchanged,
        }
    }

    /// Set Clevis information.
    pub fn set_clevis_info(self, clevis_info: ClevisInfo) -> Self {
        match self {
            EncryptionInfo::ClevisInfo(_) => EncryptionInfo::ClevisInfo(clevis_info),
            EncryptionInfo::KeyDesc(kd) => EncryptionInfo::Both(kd, clevis_info),
            EncryptionInfo::Both(kd, _) => EncryptionInfo::Both(kd, clevis_info),
        }
    }

    /// Unset the Clevis info. If only the key description is set, no action is taken.
    /// Precondition: matches!(self, EncryptionInfo::Both(_, _) | EncryptonInfo::KeyDesc(_))
    pub fn unset_clevis_info(self) -> Self {
        match self {
            EncryptionInfo::ClevisInfo(_) => {
                panic!("Cannot convert from encrypted to unencrypted device");
            }
            EncryptionInfo::Both(kd, _) => EncryptionInfo::KeyDesc(kd),
            unchanged => unchanged,
        }
    }

    /// Get an optional key description from the encryption information.
    pub fn key_description(&self) -> Option<&KeyDescription> {
        match self {
            EncryptionInfo::Both(kd, _) | EncryptionInfo::KeyDesc(kd) => Some(kd),
            _ => None,
        }
    }

    /// Get optional Clevis information from the encryption information.
    pub fn clevis_info(&self) -> Option<&ClevisInfo> {
        match self {
            EncryptionInfo::Both(_, ci) | EncryptionInfo::ClevisInfo(ci) => Some(ci),
            _ => None,
        }
    }
}

impl fmt::Display for EncryptionInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key_desc_str = if let Some(kd) = self.key_description() {
            format!("key description: \"{}\"", kd.as_application_str())
        } else {
            "no key description".to_string()
        };
        if let Some((pin, config)) = self.clevis_info() {
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
        self.key_description().hash(state);
        self.clevis_info().map(|(pin, _)| pin).hash(state);
    }
}

impl<'a> Into<Value> for &'a EncryptionInfo {
    fn into(self) -> Value {
        let mut json = Map::new();
        if let Some(kd) = self.key_description() {
            json.insert(
                "key_description".to_string(),
                Value::from(kd.as_application_str()),
            );
        }
        if let Some(info) = self.clevis_info() {
            json.insert("clevis_pin".to_string(), Value::from(info.0.to_owned()));
            json.insert("clevis_config".to_string(), info.1.clone());
        }
        Value::from(json)
    }
}

impl TryFrom<PoolEncryptionInfo> for EncryptionInfo {
    type Error = StratisError;

    fn try_from(pei: PoolEncryptionInfo) -> StratisResult<Self> {
        match pei {
            PoolEncryptionInfo::KeyDesc(MaybeInconsistent::No(kd)) => {
                Ok(EncryptionInfo::KeyDesc(kd))
            },
            PoolEncryptionInfo::ClevisInfo(MaybeInconsistent::No(ci)) => {
                Ok(EncryptionInfo::ClevisInfo(ci))
            },
            PoolEncryptionInfo::Both(MaybeInconsistent::No(kd), MaybeInconsistent::No(ci)) => {
                Ok(EncryptionInfo::Both(kd, ci))
            },
            _ => {
                Err(StratisError::Msg("Cannot determine encryption information from metadata that is inconsistent across devices in a pool".to_string()))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PoolEncryptionInfo {
    KeyDesc(MaybeInconsistent<KeyDescription>),
    ClevisInfo(MaybeInconsistent<ClevisInfo>),
    Both(
        MaybeInconsistent<KeyDescription>,
        MaybeInconsistent<ClevisInfo>,
    ),
}

impl PoolEncryptionInfo {
    /// Reconcile two PoolEncryptionInfo records.
    ///
    /// Because rollback failure can result in some devices having a key description
    /// or Clevis info and some not having that information, any reconciliation
    /// for Some(_) and None will result in Some(MaybeInconsistent::Yes).
    fn add_enc_info<I>(self, info: I) -> Self
    where
        PoolEncryptionInfo: From<I>,
    {
        let pei = PoolEncryptionInfo::from(info);
        match (self, pei) {
            (PoolEncryptionInfo::KeyDesc(kd1), PoolEncryptionInfo::KeyDesc(kd2)) => {
                if kd1 != kd2 {
                    PoolEncryptionInfo::KeyDesc(MaybeInconsistent::Yes)
                } else {
                    PoolEncryptionInfo::KeyDesc(kd1)
                }
            }
            (PoolEncryptionInfo::ClevisInfo(ci1), PoolEncryptionInfo::ClevisInfo(ci2)) => {
                if ci1 != ci2 {
                    PoolEncryptionInfo::ClevisInfo(MaybeInconsistent::Yes)
                } else {
                    PoolEncryptionInfo::ClevisInfo(ci1)
                }
            }
            (PoolEncryptionInfo::Both(kd1, ci1), PoolEncryptionInfo::Both(kd2, ci2)) => {
                PoolEncryptionInfo::Both(
                    if kd1 != kd2 {
                        MaybeInconsistent::Yes
                    } else {
                        kd1
                    },
                    if ci1 != ci2 {
                        MaybeInconsistent::Yes
                    } else {
                        ci1
                    },
                )
            }
            (PoolEncryptionInfo::KeyDesc(_), PoolEncryptionInfo::ClevisInfo(_))
            | (PoolEncryptionInfo::ClevisInfo(_), PoolEncryptionInfo::KeyDesc(_)) => {
                PoolEncryptionInfo::Both(MaybeInconsistent::Yes, MaybeInconsistent::Yes)
            }
            (PoolEncryptionInfo::KeyDesc(kd1), PoolEncryptionInfo::Both(kd2, _))
            | (PoolEncryptionInfo::Both(kd1, _), PoolEncryptionInfo::KeyDesc(kd2)) => {
                PoolEncryptionInfo::Both(
                    if kd1 != kd2 {
                        MaybeInconsistent::Yes
                    } else {
                        kd1
                    },
                    MaybeInconsistent::Yes,
                )
            }
            (PoolEncryptionInfo::ClevisInfo(ci1), PoolEncryptionInfo::Both(_, ci2))
            | (PoolEncryptionInfo::Both(_, ci1), PoolEncryptionInfo::ClevisInfo(ci2)) => {
                PoolEncryptionInfo::Both(
                    MaybeInconsistent::Yes,
                    if ci1 != ci2 {
                        MaybeInconsistent::Yes
                    } else {
                        ci1
                    },
                )
            }
        }
    }

    pub fn is_inconsistent(&self) -> bool {
        matches!(
            self,
            PoolEncryptionInfo::KeyDesc(MaybeInconsistent::Yes)
                | PoolEncryptionInfo::ClevisInfo(MaybeInconsistent::Yes)
                | PoolEncryptionInfo::Both(MaybeInconsistent::Yes, _)
                | PoolEncryptionInfo::Both(_, MaybeInconsistent::Yes)
        )
    }

    pub fn key_description(&self) -> StratisResult<Option<&KeyDescription>> {
        match self {
            PoolEncryptionInfo::KeyDesc(kd) | PoolEncryptionInfo::Both(kd, _) => {
                if let MaybeInconsistent::No(ref key_description) = kd {
                    Ok(Some(key_description))
                } else {
                    Err(StratisError::Msg(
                        "Key description is inconsistent across devices".to_string(),
                    ))
                }
            }
            _ => Ok(None),
        }
    }

    pub fn clevis_info(&self) -> StratisResult<Option<&ClevisInfo>> {
        match self {
            PoolEncryptionInfo::ClevisInfo(ci) | PoolEncryptionInfo::Both(_, ci) => {
                if let MaybeInconsistent::No(ref clevis_info) = ci {
                    Ok(Some(clevis_info))
                } else {
                    Err(StratisError::Msg(
                        "Clevis information is inconsistent across devices".to_string(),
                    ))
                }
            }
            _ => Ok(None),
        }
    }
}

impl From<&EncryptionInfo> for PoolEncryptionInfo {
    fn from(enc_info: &EncryptionInfo) -> Self {
        match enc_info {
            EncryptionInfo::KeyDesc(ref kd) => {
                PoolEncryptionInfo::KeyDesc(MaybeInconsistent::No(kd.to_owned()))
            }
            EncryptionInfo::ClevisInfo(ref ci) => {
                PoolEncryptionInfo::ClevisInfo(MaybeInconsistent::No(ci.to_owned()))
            }
            EncryptionInfo::Both(ref kd, ref ci) => PoolEncryptionInfo::Both(
                MaybeInconsistent::No(kd.to_owned()),
                MaybeInconsistent::No(ci.to_owned()),
            ),
        }
    }
}

impl<'a, I> From<I> for PoolEncryptionInfo
where
    I: IntoIterator<Item = &'a EncryptionInfo>,
{
    // Precondition: Iterator must have at least one item.
    fn from(into_iterator: I) -> Self {
        let mut iterator = into_iterator.into_iter();
        match iterator.next() {
            Some(ei) => iterator.fold(PoolEncryptionInfo::from(ei), |pool_enc_info, next| {
                pool_enc_info.add_enc_info(next)
            }),
            None => panic!("Cannot supply empty iterator"),
        }
    }
}

/// A data type respresenting a key description for the kernel keyring
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
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
            Err(StratisError::Msg(format!(
                "Key description {} contains a ';'",
                s
            )))
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
