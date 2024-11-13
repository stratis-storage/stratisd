// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{
        hash_map::{self, Entry},
        HashMap, HashSet,
    },
    fmt::{self, Debug},
    hash::{Hash, Hasher},
    slice, vec,
};

use itertools::Itertools;
use serde_json::{Map, Value};
use strum_macros::{self, EnumString, VariantNames};

use libcryptsetup_rs::SafeMemHandle;

use crate::{
    engine::{
        strat_engine::{CLEVIS_LUKS_TOKEN_ID, LUKS2_TOKEN_ID},
        types::{ClevisInfo, MaybeInconsistent},
    },
    stratis::{StratisError, StratisResult},
};

type IntoPartsTuple<'a> = (
    HashSet<&'a KeyDescription>,
    Vec<&'a ClevisInfo>,
    HashMap<u32, &'a KeyDescription>,
    HashMap<u32, &'a ClevisInfo>,
);

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
pub enum UnlockMechanism {
    KeyDesc(KeyDescription),
    ClevisInfo(ClevisInfo),
}

impl UnlockMechanism {
    pub fn is_key_desc(&self) -> bool {
        match self {
            UnlockMechanism::KeyDesc(_) => true,
            UnlockMechanism::ClevisInfo(_) => false,
        }
    }

    pub fn is_clevis_info(&self) -> bool {
        match self {
            UnlockMechanism::KeyDesc(_) => false,
            UnlockMechanism::ClevisInfo(_) => true,
        }
    }

    pub fn key_desc(&self) -> Option<&KeyDescription> {
        match self {
            UnlockMechanism::KeyDesc(kd) => Some(kd),
            UnlockMechanism::ClevisInfo(_) => None,
        }
    }

    pub fn clevis_info(&self) -> Option<&ClevisInfo> {
        match self {
            UnlockMechanism::ClevisInfo(ci) => Some(ci),
            UnlockMechanism::KeyDesc(_) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default, Serialize, Deserialize)]
pub struct InputEncryptionInfo {
    encryption_infos: Vec<(Option<u32>, UnlockMechanism)>,
}

impl InputEncryptionInfo {
    pub fn new_legacy(
        key_desc: Option<KeyDescription>,
        clevis_info: Option<ClevisInfo>,
    ) -> Option<Self> {
        let mut ei = InputEncryptionInfo {
            encryption_infos: Vec::new(),
        };
        if key_desc.is_none() && clevis_info.is_none() {
            return None;
        }
        if let Some(kd) = key_desc {
            ei.add_info(Some(LUKS2_TOKEN_ID), UnlockMechanism::KeyDesc(kd))
                .expect("Initialized new data structure");
        }
        if let Some(ci) = clevis_info {
            ei.add_info(Some(CLEVIS_LUKS_TOKEN_ID), UnlockMechanism::ClevisInfo(ci))
                .expect("Initialized new data structure");
        }
        Some(ei)
    }

    pub fn new(
        kds: Vec<(Option<u32>, KeyDescription)>,
        cis: Vec<(Option<u32>, ClevisInfo)>,
    ) -> StratisResult<Option<Self>> {
        if kds.is_empty() && cis.is_empty() {
            Ok(None)
        } else {
            let mut iei = InputEncryptionInfo {
                encryption_infos: Vec::default(),
            };
            for (slot, kd) in kds {
                iei.add_info(slot, UnlockMechanism::KeyDesc(kd))?;
            }
            for (slot, ci) in cis {
                iei.add_info(slot, UnlockMechanism::ClevisInfo(ci))?;
            }
            Ok(Some(iei))
        }
    }

    pub fn is_empty(&self) -> bool {
        self.encryption_infos.is_empty()
    }

    pub fn add_info(
        &mut self,
        token_slot: Option<u32>,
        mech: UnlockMechanism,
    ) -> StratisResult<()> {
        if let Some(t) = token_slot {
            if self.iter().map(|(slot, _)| slot).contains(&Some(t)) {
                return Err(StratisError::Msg(format!("Token slot {t} already taken")));
            }
        }
        self.encryption_infos.push((token_slot, mech));
        Ok(())
    }

    pub fn iter(&self) -> slice::Iter<'_, (Option<u32>, UnlockMechanism)> {
        self.encryption_infos.iter()
    }

    pub fn into_parts_legacy(
        &self,
    ) -> StratisResult<(Option<&KeyDescription>, Option<&ClevisInfo>)> {
        let mut kd = None;
        let mut ci = None;
        for (token_slot, unlock_mech) in self.encryption_infos.iter() {
            match token_slot {
                None => {
                    return Err(StratisError::Msg(
                        "Static token slots are required for V1 of the metadata".to_string(),
                    ));
                }
                Some(LUKS2_TOKEN_ID) => match unlock_mech {
                    UnlockMechanism::KeyDesc(k) => kd = Some(k),
                    UnlockMechanism::ClevisInfo(_) => {
                        return Err(StratisError::Msg(format!(
                            "Token slot {LUKS2_TOKEN_ID} is required to refer to a key description in V1 of the metadata"
                        )));
                    }
                },
                Some(CLEVIS_LUKS_TOKEN_ID) => match unlock_mech {
                    UnlockMechanism::ClevisInfo(c) => ci = Some(c),
                    UnlockMechanism::KeyDesc(_) => {
                        return Err(StratisError::Msg(format!(
                            "Token slot {CLEVIS_LUKS_TOKEN_ID} is required to refer to Clevis info in V1 of the metadata"
                        )));
                    }
                },
                Some(t) => {
                    return Err(StratisError::Msg(format!(
                        "Invalid token slot {t} for V1 of the metadata"
                    )));
                }
            }
        }

        Ok((kd, ci))
    }

    pub fn into_parts(&self) -> StratisResult<IntoPartsTuple<'_>> {
        let mut key_descs = HashSet::default();
        let mut clevis_infos = Vec::default();
        let mut key_descs_with_token_id = HashMap::default();
        let mut clevis_infos_with_token_id = HashMap::default();
        for (token_slot, unlock_mechanism) in self.iter() {
            match token_slot {
                Some(t) => match unlock_mechanism {
                    UnlockMechanism::KeyDesc(kd) => {
                        if key_descs_with_token_id.insert(*t, kd).is_some() {
                            return Err(StratisError::Msg(format!(
                                "Token slot {t} is requested for two unlock mechanisms"
                            )));
                        }
                    }
                    UnlockMechanism::ClevisInfo(ci) => {
                        if clevis_infos_with_token_id.insert(*t, ci).is_some() {
                            return Err(StratisError::Msg(format!(
                                "Token slot {t} is requested for two unlock mechanisms"
                            )));
                        }
                    }
                },
                None => match unlock_mechanism {
                    UnlockMechanism::KeyDesc(kd) => {
                        key_descs.insert(kd);
                    }
                    UnlockMechanism::ClevisInfo(ci) => {
                        clevis_infos.push(ci);
                    }
                },
            }
        }

        Ok((
            key_descs,
            clevis_infos,
            key_descs_with_token_id,
            clevis_infos_with_token_id,
        ))
    }
}

impl From<EncryptionInfo> for InputEncryptionInfo {
    fn from(ei: EncryptionInfo) -> Self {
        InputEncryptionInfo {
            encryption_infos: ei
                .into_iter()
                .map(|(slot, um)| (Some(slot), um))
                .collect::<Vec<_>>(),
        }
    }
}

impl IntoIterator for InputEncryptionInfo {
    type Item = (Option<u32>, UnlockMechanism);
    type IntoIter = vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.encryption_infos.into_iter()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct EncryptionInfo {
    encryption_infos: HashMap<u32, UnlockMechanism>,
}

impl EncryptionInfo {
    pub fn new() -> Self {
        EncryptionInfo {
            encryption_infos: HashMap::default(),
        }
    }

    /// Note: This is a legacy method for operations that only operate on one token per type of
    /// unlock mechanism.
    pub fn from_options(kd: Option<KeyDescription>, ci: Option<ClevisInfo>) -> Option<Self> {
        match (kd, ci) {
            (Some(k), Some(c)) => {
                let mut ei = EncryptionInfo::new();
                let _ = ei.add_info(LUKS2_TOKEN_ID, UnlockMechanism::KeyDesc(k));
                let _ = ei.add_info(CLEVIS_LUKS_TOKEN_ID, UnlockMechanism::ClevisInfo(c));
                Some(ei)
            }
            (Some(k), _) => {
                let mut ei = EncryptionInfo::new();
                let _ = ei.add_info(LUKS2_TOKEN_ID, UnlockMechanism::KeyDesc(k));
                Some(ei)
            }
            (_, Some(c)) => {
                let mut ei = EncryptionInfo::new();
                let _ = ei.add_info(CLEVIS_LUKS_TOKEN_ID, UnlockMechanism::ClevisInfo(c));
                Some(ei)
            }
            (_, _) => None,
        }
    }

    pub fn single_key_description(&self) -> Option<(u32, &KeyDescription)> {
        self.encryption_infos
            .iter()
            .filter_map(|(i, mech)| mech.key_desc().map(|kd| (i, kd)))
            .min_by(|(i, _), (j, _)| i.cmp(j))
            .map(|(i, kd)| (*i, kd))
    }

    pub fn single_clevis_info(&self) -> Option<(u32, &ClevisInfo)> {
        self.encryption_infos
            .iter()
            .filter_map(|(i, mech)| mech.clevis_info().map(|ci| (i, ci)))
            .min_by(|(i, _), (j, _)| i.cmp(j))
            .map(|(i, clevis)| (*i, clevis))
    }

    pub fn all_token_slots(&self) -> impl Iterator<Item = u32> + '_ {
        self.encryption_infos.keys().cloned()
    }

    pub fn free_token_slot(&self) -> u32 {
        // Find the maximum value that's occupied and add 1 to it for all in case there is no unused token
        // slot smaller than or equal to the max.
        let occupied = self.all_token_slots().collect::<HashSet<_>>();
        let max = occupied.iter().max().cloned().unwrap_or(0);
        let all = (0..=max + 1).collect::<HashSet<_>>();
        *all.difference(&occupied)
            .next()
            .expect("Difference should always contain one element")
    }

    pub fn all_key_descriptions(&self) -> impl Iterator<Item = (&u32, &KeyDescription)> + '_ {
        self.encryption_infos
            .iter()
            .filter_map(|(i, mech)| mech.key_desc().map(|kd| (i, kd)))
    }

    pub fn all_clevis_infos(&self) -> impl Iterator<Item = (&u32, &ClevisInfo)> + '_ {
        self.encryption_infos
            .iter()
            .filter_map(|(i, mech)| mech.clevis_info().map(|clevis| (i, clevis)))
    }

    pub fn all_infos(&self) -> impl Iterator<Item = (&u32, &UnlockMechanism)> + '_ {
        self.encryption_infos.iter()
    }

    pub fn add_info(&mut self, token_slot: u32, mech: UnlockMechanism) -> StratisResult<()> {
        if let Entry::Vacant(v) = self.encryption_infos.entry(token_slot) {
            v.insert(mech);
            Ok(())
        } else {
            Err(StratisError::Msg(format!(
                "Token slot {token_slot} already occupied"
            )))
        }
    }

    pub fn get_info(&self, token_slot: u32) -> Option<&UnlockMechanism> {
        self.encryption_infos.get(&token_slot)
    }

    pub fn set_info(&mut self, token_slot: u32, mech: UnlockMechanism) -> StratisResult<()> {
        let entry = self.encryption_infos.entry(token_slot);
        match entry {
            Entry::Occupied(mut entry) => match entry.get_mut() {
                UnlockMechanism::KeyDesc(ref mut kd) => match mech {
                    UnlockMechanism::KeyDesc(kd_set) => *kd = kd_set,
                    UnlockMechanism::ClevisInfo(_) => return Err(StratisError::Msg("Binding is a key description but provided unlock mechanism is a Clevis binding".to_string())),
                },
                UnlockMechanism::ClevisInfo(ref mut clevis) => match mech {
                    UnlockMechanism::KeyDesc(_) => return Err(StratisError::Msg("Binding is a Clevis binding but provided unlock mechanism is a key description".to_string())),
                    UnlockMechanism::ClevisInfo(clevis_set) => *clevis = clevis_set,
                },
            }
            Entry::Vacant(entry) => {
                entry.insert(mech);
            }
        }
        Ok(())
    }

    pub fn remove(&mut self, token_slot: u32) -> bool {
        self.encryption_infos.remove(&token_slot).is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.encryption_infos.is_empty()
    }

    /// Returns the values that are in &self but not in &other.
    pub fn diff(&self, other: &Self) -> Option<u32> {
        self.encryption_infos
            .keys()
            .cloned()
            .collect::<HashSet<_>>()
            .difference(
                &other
                    .encryption_infos
                    .keys()
                    .cloned()
                    .collect::<HashSet<_>>(),
            )
            .next()
            .cloned()
    }
}

impl IntoIterator for EncryptionInfo {
    type Item = (u32, UnlockMechanism);
    type IntoIter = hash_map::IntoIter<u32, UnlockMechanism>;

    fn into_iter(self) -> Self::IntoIter {
        self.encryption_infos.into_iter()
    }
}

impl fmt::Display for EncryptionInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            self.encryption_infos
                .iter()
                .map(|(i, mech)| match mech {
                    UnlockMechanism::KeyDesc(kd) => {
                        format!("{i}: key description {}", kd.as_application_str())
                    }
                    UnlockMechanism::ClevisInfo((pin, json)) => {
                        format!("{i}: clevis info {pin}, {json}")
                    }
                })
                .join(", ")
        )
    }
}

// Implement Hash explicitly because Value does not implement Hash.
// The reason Value does not implement Hash is that some JSON is really big
// and hashing it would be expensive. This JSON probably won't be, but it
// serves no obvious purpose to stringify it and then hash it.
// Necessary Hash Property: \forall x_1, x_2 in EncryptionInfo,
// if x_1 == x_2, then hash(x_1) == hash(x_2) obviously holds.
impl Hash for EncryptionInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for (t, mech) in self.encryption_infos.iter() {
            t.hash(state);
            match mech {
                UnlockMechanism::KeyDesc(kd) => kd.hash(state),
                UnlockMechanism::ClevisInfo((pin, _)) => pin.hash(state),
            }
        }
    }
}

impl Into<Value> for &EncryptionInfo {
    fn into(self) -> Value {
        let json = self
            .encryption_infos
            .iter()
            .map(|(token_slot, mech)| {
                (
                    token_slot.to_string(),
                    match mech {
                        UnlockMechanism::KeyDesc(kd) => Value::from(kd.as_application_str()),
                        UnlockMechanism::ClevisInfo((pin, config)) => {
                            Value::from(vec![Value::from(pin.to_owned()), config.to_owned()])
                        }
                    },
                )
            })
            .collect::<Map<_, _>>();
        Value::from(json)
    }
}

impl TryFrom<PoolEncryptionInfo> for EncryptionInfo {
    type Error = StratisError;

    fn try_from(pei: PoolEncryptionInfo) -> StratisResult<Self> {
        match pei {
            PoolEncryptionInfo::KeyDesc(MaybeInconsistent::No(kd)) => {
                let mut ei = EncryptionInfo::new();
                ei.add_info(LUKS2_TOKEN_ID, UnlockMechanism::KeyDesc(kd))?;
                Ok(ei)
            },
            PoolEncryptionInfo::ClevisInfo(MaybeInconsistent::No(ci)) => {
                let mut ei = EncryptionInfo::new();
                ei.add_info(CLEVIS_LUKS_TOKEN_ID, UnlockMechanism::ClevisInfo(ci))?;
                Ok(ei)
            },
            PoolEncryptionInfo::Both(MaybeInconsistent::No(kd), MaybeInconsistent::No(ci)) => {
                let mut ei = EncryptionInfo::new();
                ei.add_info(LUKS2_TOKEN_ID, UnlockMechanism::KeyDesc(kd))?;
                ei.add_info(CLEVIS_LUKS_TOKEN_ID, UnlockMechanism::ClevisInfo(ci))?;
                Ok(ei)
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
        let key_description = enc_info.single_key_description();
        let clevis_info = enc_info.single_clevis_info();
        match (key_description, clevis_info) {
            (Some((_, kd)), None) => {
                PoolEncryptionInfo::KeyDesc(MaybeInconsistent::No(kd.to_owned()))
            }
            (None, Some((_, ci))) => {
                PoolEncryptionInfo::ClevisInfo(MaybeInconsistent::No(ci.to_owned()))
            }
            (Some((_, kd)), Some((_, ci))) => PoolEncryptionInfo::Both(
                MaybeInconsistent::No(kd.to_owned()),
                MaybeInconsistent::No(ci.to_owned()),
            ),
            (_, _) => unreachable!("Cannot have no unlock mechanism"),
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

/// A data type representing a key description for the kernel keyring
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
                "Key description {s} contains a ';'"
            )))
        } else {
            Ok(KeyDescription(s))
        }
    }
}

impl TryFrom<&String> for KeyDescription {
    type Error = StratisError;

    fn try_from(s: &String) -> StratisResult<KeyDescription> {
        KeyDescription::try_from(s.to_owned())
    }
}

/// Use Clevis or keyring to unlock LUKS volume.
#[derive(Serialize, Deserialize, Clone, Copy, Eq, PartialEq, Debug, EnumString, VariantNames)]
#[strum(serialize_all = "snake_case")]
pub enum UnlockMethod {
    Clevis,
    Keyring,
    Any,
}

/// Represents the token slot input for an encryption operation.
#[derive(PartialEq, Serialize, Deserialize)]
pub enum OptionalTokenSlotInput {
    /// Indicates that legacy behavior of assuming one token slot for a key description and one for
    /// Clevis info should be used.
    Legacy,
    /// No token slot is provided. Typically means that one should be automatically selected.
    None,
    /// A specific token slot to use for the operation.
    Some(u32),
}

/// Represents the token slot input for an unlock operation.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum TokenUnlockMethod {
    None,
    Any,
    Token(u32),
}

impl TokenUnlockMethod {
    pub fn from_options(input: Option<Option<u32>>) -> Self {
        match input {
            None => TokenUnlockMethod::None,
            Some(None) => TokenUnlockMethod::Any,
            Some(Some(t)) => TokenUnlockMethod::Token(t),
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, TokenUnlockMethod::None)
    }

    pub fn is_some(&self) -> bool {
        !matches!(self, TokenUnlockMethod::None)
    }

    pub fn is_any(&self) -> bool {
        matches!(self, TokenUnlockMethod::Any)
    }

    pub fn get_token_slot(&self) -> StratisResult<Option<u32>> {
        match self {
            TokenUnlockMethod::None => Err(StratisError::Msg(
                "No unlock mechanism was provided".to_string(),
            )),
            TokenUnlockMethod::Any => Ok(None),
            TokenUnlockMethod::Token(t) => Ok(Some(*t)),
        }
    }
}

impl From<Option<UnlockMethod>> for TokenUnlockMethod {
    fn from(method: Option<UnlockMethod>) -> Self {
        match method {
            Some(UnlockMethod::Any) => TokenUnlockMethod::Any,
            Some(UnlockMethod::Clevis) => TokenUnlockMethod::Token(CLEVIS_LUKS_TOKEN_ID),
            Some(UnlockMethod::Keyring) => TokenUnlockMethod::Token(LUKS2_TOKEN_ID),
            None => TokenUnlockMethod::None,
        }
    }
}
