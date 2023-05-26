// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Types representing known information about devices

use std::{
    collections::{hash_map, HashMap, HashSet},
    fmt,
    hash::{Hash, Hasher},
    path::Path,
};

use either::Either;
use serde_json::Value;

use crate::{
    engine::{
        shared::{gather_encryption_info, gather_pool_name},
        strat_engine::{
            backstore::StratBlockDev,
            liminal::{
                identify::{DeviceInfo, LuksInfo, StratisDevInfo, StratisInfo},
                setup::get_name,
            },
            metadata::{StratisIdentifiers, BDA},
        },
        types::{
            DevUuid, EncryptionInfo, LockedPoolInfo, MaybeInconsistent, Name, PoolDevice,
            PoolEncryptionInfo, PoolUuid, StoppedPoolInfo,
        },
    },
    stratis::StratisResult,
};

/// Info for a discovered LUKS device belonging to Stratis.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct LLuksInfo {
    pub dev_info: StratisDevInfo,
    pub identifiers: StratisIdentifiers,
    pub encryption_info: EncryptionInfo,
    pub pool_name: Option<Name>,
}

impl fmt::Display for LLuksInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}, {}, {}",
            self.dev_info, self.identifiers, self.encryption_info,
        )?;
        if let Some(ref pn) = self.pool_name {
            write!(f, ", {}", pn)
        } else {
            Ok(())
        }
    }
}

impl From<LuksInfo> for LLuksInfo {
    fn from(info: LuksInfo) -> LLuksInfo {
        LLuksInfo {
            dev_info: info.dev_info,
            identifiers: info.identifiers,
            encryption_info: info.encryption_info,
            pool_name: info.pool_name,
        }
    }
}

impl<'a> Into<Value> for &'a LLuksInfo {
    // Precondition: (&StratisInfo).into() pattern matches Value::Object()
    // Precondition: (&EncryptionInfo).into() pattern matches Value::Object()
    fn into(self) -> Value {
        let mut json = <&StratisDevInfo as Into<Value>>::into(&self.dev_info);
        let map = json
            .as_object_mut()
            .expect("StratisInfo conversion returns a JSON object");
        map.extend(
            if let Value::Object(id_map) =
                <&StratisIdentifiers as Into<Value>>::into(&self.identifiers)
            {
                id_map.into_iter()
            } else {
                unreachable!("StratisIdentifiers conversion returns a JSON object");
            },
        );
        map.extend(
            if let Value::Object(enc_map) =
                <&EncryptionInfo as Into<Value>>::into(&self.encryption_info)
            {
                enc_map.into_iter()
            } else {
                unreachable!("EncryptionInfo conversion returns a JSON object");
            },
        );
        if let Some(ref n) = self.pool_name {
            map.insert("pool_name".to_string(), Value::from(n.to_string()));
        }
        json
    }
}

/// Convert a reference to a HashMap to an owned HashMap with borrowed values.
pub fn stratis_infos_ref(
    infos: &HashMap<DevUuid, LStratisInfo>,
) -> HashMap<DevUuid, &LStratisInfo> {
    infos
        .iter()
        .map(|(dev_uuid, info)| (*dev_uuid, info))
        .collect::<HashMap<_, _>>()
}

/// Split a `Vec<LStratisInfo>` into a `Vec` of BDAs and a `Vec` of the remaining
/// information contained by `Vec<LStratisInfo>`.
pub fn split_stratis_infos(
    infos: HashMap<DevUuid, LStratisInfo>,
) -> (HashMap<DevUuid, LStratisDevInfo>, HashMap<DevUuid, BDA>) {
    infos.into_iter().fold(
        (HashMap::new(), HashMap::new()),
        |(mut new_infos, mut bdas), (dev_uuid, info)| {
            new_infos.insert(
                dev_uuid,
                LStratisDevInfo {
                    dev_info: info.dev_info,
                    luks: info.luks,
                },
            );
            bdas.insert(dev_uuid, info.bda);
            (new_infos, bdas)
        },
    )
}

/// Merge BDAs with the rest of the device information.
///
/// Precondition: infos.keys() == bdas.keys()
pub fn reconstruct_stratis_infos(
    mut infos: HashMap<DevUuid, LStratisDevInfo>,
    mut bdas: HashMap<DevUuid, BDA>,
) -> DeviceSet {
    let uuids = infos.keys().copied().collect::<HashSet<_>>();
    assert_eq!(uuids, bdas.keys().copied().collect::<HashSet<_>>());

    uuids
        .into_iter()
        .map(|uuid| {
            let info = infos.remove(&uuid).expect("infos.keys() == bdas.keys()");
            let bda = bdas.remove(&uuid).expect("infos.keys() == bdas.keys()");
            (
                uuid,
                LInfo::Stratis(LStratisInfo {
                    dev_info: info.dev_info,
                    luks: info.luks,
                    bda,
                }),
            )
        })
        .collect::<DeviceSet>()
}

/// Info for a Stratis device.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct LStratisDevInfo {
    pub dev_info: StratisDevInfo,
    /// Luks information. The information will be set if this is a Stratis
    /// device which is an activated encrypted device, and will be
    /// information about that physical device.
    pub luks: Option<LLuksInfo>,
}

impl From<LStratisInfo> for (LStratisDevInfo, BDA) {
    fn from(info: LStratisInfo) -> Self {
        (
            LStratisDevInfo {
                dev_info: info.dev_info,
                luks: info.luks,
            },
            info.bda,
        )
    }
}

/// Info for a Stratis device.
#[derive(Debug)]
pub struct LStratisInfo {
    pub dev_info: StratisDevInfo,
    pub bda: BDA,
    /// Luks information. The information will be set if this is a Stratis
    /// device which is an activated encrypted device, and will be
    /// information about that physical device.
    pub luks: Option<LLuksInfo>,
}

impl PartialEq for LStratisInfo {
    fn eq(&self, rhs: &Self) -> bool {
        self.bda.identifiers() == rhs.bda.identifiers() && self.dev_info == rhs.dev_info
    }
}

impl Eq for LStratisInfo {}

impl Hash for LStratisInfo {
    fn hash<H>(&self, hasher: &mut H)
    where
        H: Hasher,
    {
        self.bda.identifiers().hash(hasher);
        self.dev_info.hash(hasher);
    }
}

impl fmt::Display for LStratisInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(info) = &self.luks {
            write!(
                f,
                "logical device with {}, {} and physical device with {}",
                self.bda.identifiers(),
                self.dev_info,
                info
            )
        } else {
            write!(f, "{}, {}", self.bda.identifiers(), self.dev_info)
        }
    }
}

impl From<StratisInfo> for LStratisInfo {
    fn from(info: StratisInfo) -> LStratisInfo {
        LStratisInfo {
            dev_info: info.dev_info,
            bda: info.bda,
            luks: None,
        }
    }
}

impl<'a> Into<Value> for &'a LStratisInfo {
    // Precondition: (&StratisInfo).into() pattern matches Value::Object()
    // Precondition: (&LLuksInfo).into() pattern matches Value::Object()
    fn into(self) -> Value {
        let mut json = self
            .luks
            .as_ref()
            .map(|luks| json!({ "luks": <&LLuksInfo as Into<Value>>::into(luks) }))
            .unwrap_or_else(|| json!({}));
        if let Value::Object(ref mut map) = json {
            map.extend(
                if let Value::Object(map) = <&StratisDevInfo as Into<Value>>::into(&self.dev_info) {
                    map.into_iter()
                } else {
                    unreachable!("StratisInfo conversion returns a JSON object");
                },
            );
            map.extend(
                if let Value::Object(map) =
                    <&StratisIdentifiers as Into<Value>>::into(&self.bda.identifiers())
                {
                    map.into_iter()
                } else {
                    unreachable!("StratisInfo conversion returns a JSON object");
                },
            );
        } else {
            unreachable!("json!() always creates a JSON object");
        };
        json
    }
}

impl LStratisInfo {
    #[allow(dead_code)]
    fn invariant(&self) {
        assert!(match &self.luks {
            None => true,
            Some(luks) =>
                luks.identifiers == self.bda.identifiers()
                    && luks.dev_info.devnode != self.dev_info.devnode
                    && luks.dev_info.device_number != self.dev_info.device_number,
        });
    }
}

/// A unifying Info struct for Stratis or Luks devices. This struct is used
/// for storing known information in DeviceSet and DeviceBag.
#[derive(Debug, Eq, Hash, PartialEq)]
pub enum LInfo {
    /// A Stratis device, which may be an encrypted device
    Stratis(LStratisInfo),
    /// A LUKS device
    Luks(LLuksInfo),
}

impl fmt::Display for LInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LInfo::Stratis(info) => write!(f, "Stratis device with {info}"),
            LInfo::Luks(info) => write!(f, "LUKS device belonging to Stratis with {info}"),
        }
    }
}

impl From<DeviceInfo> for LInfo {
    fn from(info: DeviceInfo) -> LInfo {
        match info {
            DeviceInfo::Luks(info) => LInfo::Luks(info.into()),
            DeviceInfo::Stratis(info) => LInfo::Stratis(info.into()),
        }
    }
}

impl<'a> Into<Value> for &'a LInfo {
    // Precondition: (&LStratisInfo).into() pattern matches Value::Object()
    // Precondition: (&LLuksInfo).into() pattern matches Value::Object()
    fn into(self) -> Value {
        match self {
            LInfo::Stratis(info) => info.into(),
            LInfo::Luks(info) => info.into(),
        }
    }
}

impl LInfo {
    pub fn stratis_identifiers(&self) -> StratisIdentifiers {
        match self {
            LInfo::Luks(info) => info.identifiers,
            LInfo::Stratis(info) => info.bda.identifiers(),
        }
    }

    fn encryption_info(&self) -> Option<&EncryptionInfo> {
        match self {
            LInfo::Luks(info) => Some(&info.encryption_info),
            LInfo::Stratis(info) => info.luks.as_ref().map(|i| &i.encryption_info),
        }
    }

    /// Returns true if the data represents a device with encryption managed
    /// by Stratis, otherwise false.
    pub fn is_encrypted(&self) -> bool {
        self.encryption_info().is_some()
    }

    /// Returns true if the data represents a device with encryption managed
    /// by Stratis which is not opened.
    pub fn is_closed(&self) -> bool {
        match self {
            LInfo::Luks(_) => true,
            LInfo::Stratis(_) => false,
        }
    }

    /// Remove a device if it matches the given device path.
    fn update_on_remove(
        info: LInfo,
        path: &Path,
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
    ) -> Option<LInfo> {
        match info {
            LInfo::Luks(linfo) => {
                if linfo.dev_info.devnode == path {
                    None
                } else {
                    warn!("Device with pool UUID {}, device UUID {} appears to have been removed but the path did not match the known Stratis device with these identifiers", pool_uuid, dev_uuid);
                    Some(LInfo::Luks(linfo))
                }
            }
            LInfo::Stratis(sinfo) => {
                if Some(path) == sinfo.luks.as_ref().map(|i| i.dev_info.devnode.as_path()) {
                    info!("Encrypted backing device with pool UUID {}, device UUID {} is no longer available; removing activated devicemapper device as well", pool_uuid, dev_uuid);
                    None
                } else if path == sinfo.dev_info.devnode {
                    if let Some(l) = sinfo.luks {
                        info!("Encrypted Stratis device with pool UUID {}, device UUID {} is no longer available; marking encrypted backing device as closed", pool_uuid, dev_uuid);
                        Some(LInfo::Luks(l))
                    } else {
                        info!("Stratis device with pool UUID {}, device UUID {} is no longer available", pool_uuid, dev_uuid);
                        None
                    }
                } else {
                    warn!("Device with pool UUID {}, device UUID {} appears to have been removed but the path did not match the known Stratis device with these identifiers", pool_uuid, dev_uuid);
                    Some(LInfo::Stratis(sinfo))
                }
            }
        }
    }

    // Combine two devices which have identical pool and device UUIDs.
    // The first argument is the older information, the second the newer.
    // Allow the newer information to supplant the older.
    fn update(info_1: LInfo, info_2: DeviceInfo) -> Either<LInfo, LInfo> {
        // Returns true if the information found via udev for two devices is
        // compatible, otherwise false.
        // Precondition: Stratis identifiers of devices are the same
        fn luks_luks_compatible(info_1: &LLuksInfo, info_2: &LuksInfo) -> bool {
            assert_eq!(info_1.identifiers, info_2.identifiers);
            if info_1.dev_info.device_number == info_2.dev_info.device_number
                && info_1.encryption_info == info_2.encryption_info
            {
                true
            } else {
                warn!(
                    "LUKS info conflicts: {}, {} conflicts with {}, {}",
                    info_1.dev_info.device_number,
                    info_1.encryption_info,
                    info_2.dev_info.device_number,
                    info_2.encryption_info
                );
                false
            }
        }

        // Returns true if the information found via udev for two devices is
        // compatible, otherwise false.
        // Precondition: Stratis identifiers of devices are the same
        fn stratis_stratis_compatible(info_1: &LStratisInfo, info_2: &StratisInfo) -> bool {
            assert_eq!(info_1.bda.identifiers(), info_2.bda.identifiers());
            if info_1.dev_info.device_number == info_2.dev_info.device_number {
                true
            } else {
                warn!(
                    "Stratis info conflicts: {} conflicts with {}",
                    info_1.dev_info.device_number, info_2.dev_info.device_number
                );
                false
            }
        }

        match (info_1, info_2) {
            (LInfo::Luks(luks_info), DeviceInfo::Stratis(strat_info)) => {
                Either::Left(LInfo::Stratis(LStratisInfo {
                    dev_info: strat_info.dev_info,
                    bda: strat_info.bda,
                    luks: Some(luks_info),
                }))
            }
            (LInfo::Stratis(strat_info), DeviceInfo::Luks(luks_info)) => {
                if let Some(luks) = strat_info.luks.as_ref() {
                    if !luks_luks_compatible(luks, &luks_info) {
                        return Either::Right(LInfo::Stratis(strat_info));
                    }
                }
                Either::Left(LInfo::Stratis(LStratisInfo {
                    dev_info: strat_info.dev_info,
                    bda: strat_info.bda,
                    luks: Some(LLuksInfo::from(luks_info)),
                }))
            }
            (LInfo::Luks(luks_info_1), DeviceInfo::Luks(luks_info_2)) => {
                if !luks_luks_compatible(&luks_info_1, &luks_info_2) {
                    Either::Right(LInfo::Luks(luks_info_1))
                } else {
                    Either::Left(LInfo::Luks(LLuksInfo::from(luks_info_2)))
                }
            }
            (LInfo::Stratis(strat_info_1), DeviceInfo::Stratis(strat_info_2)) => {
                if !stratis_stratis_compatible(&strat_info_1, &strat_info_2) {
                    Either::Right(LInfo::Stratis(strat_info_1))
                } else {
                    Either::Left(LInfo::Stratis(LStratisInfo {
                        dev_info: strat_info_2.dev_info,
                        bda: strat_info_2.bda,
                        luks: strat_info_1.luks,
                    }))
                }
            }
        }
    }
}

/// An iterator for the DeviceSet
pub struct Iter<'a> {
    items: hash_map::Iter<'a, DevUuid, LInfo>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a DevUuid, &'a LInfo);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.items.next()
    }
}

/// A set of devices, each distinguished by its unique device UUID.
#[derive(Debug, Eq, PartialEq)]
pub struct DeviceSet {
    internal: HashMap<DevUuid, LInfo>,
}

impl Default for DeviceSet {
    fn default() -> DeviceSet {
        DeviceSet::new()
    }
}

impl FromIterator<(DevUuid, LInfo)> for DeviceSet {
    fn from_iter<I>(i: I) -> Self
    where
        I: IntoIterator<Item = (DevUuid, LInfo)>,
    {
        DeviceSet {
            internal: HashMap::from_iter(i),
        }
    }
}

impl IntoIterator for DeviceSet {
    type Item = <HashMap<DevUuid, LInfo> as IntoIterator>::Item;
    type IntoIter = <HashMap<DevUuid, LInfo> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.internal.into_iter()
    }
}

impl From<Vec<StratBlockDev>> for DeviceSet {
    fn from(vec: Vec<StratBlockDev>) -> Self {
        vec.into_iter()
            .flat_map(|bd| {
                let dev_uuid = bd.uuid();
                Vec::<DeviceInfo>::from(bd)
                    .into_iter()
                    .map(move |info| (dev_uuid, LInfo::from(info)))
            })
            .collect::<DeviceSet>()
    }
}

impl DeviceSet {
    /// Create a new, empty DeviceSet
    pub fn new() -> DeviceSet {
        DeviceSet {
            internal: HashMap::new(),
        }
    }

    /// An iterator over the elements in the set
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            items: self.internal.iter(),
        }
    }

    /// Returns true if some of the devices are encrypted and closed.
    pub fn some_closed(&self) -> bool {
        self.internal.iter().any(|(_, info)| info.is_closed())
    }

    /// Return a view of the DeviceSet as a set of wholly opened devices.
    /// Return None if some of the devices are unopened.
    fn as_opened_set(&self) -> Option<HashMap<DevUuid, &LStratisInfo>> {
        if self.some_closed() {
            None
        } else {
            Some(
                self.internal
                    .iter()
                    .map(|(dev_uuid, info)| match info {
                        LInfo::Luks(_) => unreachable!("!self.some_closed() is satisfied"),
                        LInfo::Stratis(info) => (*dev_uuid, info),
                    })
                    .collect(),
            )
        }
    }

    /// Return a view of the DeviceSet as a set of wholly opened devices.
    /// Return Either::Right(_) if some of the devices are unopened.
    pub fn into_opened_set(self) -> Either<HashMap<DevUuid, LStratisInfo>, Self> {
        if self.some_closed() {
            Either::Right(self)
        } else {
            Either::Left(
                self.internal
                    .into_iter()
                    .map(|(dev_uuid, info)| match info {
                        LInfo::Luks(_) => unreachable!("!self.some_closed() is satisfied"),
                        LInfo::Stratis(info) => (dev_uuid, info),
                    })
                    .collect(),
            )
        }
    }

    /// Consumes the contents of the device set, returning a DeviceBag
    pub fn into_bag(mut self) -> DeviceBag {
        DeviceBag {
            internal: self.internal.drain().map(|(_, info)| info).collect(),
        }
    }

    /// The unique encryption info for this set.
    pub fn encryption_info(&self) -> StratisResult<Option<PoolEncryptionInfo>> {
        gather_encryption_info(
            self.internal.len(),
            self.internal.values().map(|info| info.encryption_info()),
        )
    }

    /// Get the name, if available, of the pool formed by the devices
    /// in this DeviceSet.
    pub fn pool_name(&self) -> StratisResult<MaybeInconsistent<Option<Name>>> {
        match self.as_opened_set() {
            Some(set) => get_name(set).map(MaybeInconsistent::No),
            None => gather_pool_name(
                self.internal.len(),
                self.internal.values().map(|info| match info {
                    LInfo::Stratis(s) => s.luks.as_ref().map(|l| l.pool_name.as_ref()),
                    LInfo::Luks(l) => Some(l.pool_name.as_ref()),
                }),
            )
            .map(|opt| opt.expect("self.as_opened_set().is_some() if pool is unencrypted")),
        }
    }

    /// The encryption information and devices registered for locked pools to be
    /// exported over the API. If none of the infos correspond to a Stratis managed
    /// encrypted device, None.
    ///
    /// This method filters out Stratis devices that have no detected associated
    /// LUKS2 device. This could happen for encrypted devices if the LUKS2 device
    /// is detected after the unlocked Stratis device but should eventually become
    /// consistent.
    ///
    /// Error from gather_encryption_info is converted into an option because
    /// unlocked Stratis devices and LUKS2 devices on which the Stratis devices are
    /// stored may appear at different times in udev. This is not necessarily
    /// an error case and may resolve itself after more devices appear in udev.
    pub fn locked_pool_info(&self) -> Option<LockedPoolInfo> {
        gather_encryption_info(
            self.internal.len(),
            self.internal.values().map(|info| info.encryption_info()),
        )
        .ok()
        .and_then(|info| info)
        .and_then(|info| {
            self.internal
                .iter()
                .map(|(uuid, l)| {
                    let devnode = match l {
                        LInfo::Stratis(strat_info) => {
                            strat_info.luks.as_ref().map(|l| l.dev_info.devnode.clone())
                        }
                        LInfo::Luks(luks_info) => Some(luks_info.dev_info.devnode.clone()),
                    };
                    devnode.map(|devnode| PoolDevice {
                        devnode,
                        uuid: *uuid,
                    })
                })
                .fold(Some(Vec::new()), |vec, dev_info| {
                    vec.and_then(|mut v| {
                        dev_info.map(|d| {
                            v.push(d);
                            v
                        })
                    })
                })
                .map(|d| LockedPoolInfo {
                    info: info.clone(),
                    devices: d,
                })
        })
    }

    /// The encryption information and devices registered for stopped pools to
    /// be exported over the API.
    ///
    /// Converts an error result from gather_encryption_info into maximum
    /// uncertainty, since the error means that encryption information is
    /// completely missing from one device.
    pub fn stopped_pool_info(&self) -> StoppedPoolInfo {
        let info = gather_encryption_info(
            self.internal.len(),
            self.internal.values().map(|info| info.encryption_info()),
        )
        .unwrap_or({
            Some(PoolEncryptionInfo::Both(
                MaybeInconsistent::Yes,
                MaybeInconsistent::Yes,
            ))
        });

        StoppedPoolInfo {
            info,
            devices: self
                .internal
                .iter()
                .map(|(uuid, l)| {
                    let devnode = match l {
                        LInfo::Stratis(strat_info) => strat_info
                            .luks
                            .as_ref()
                            .map(|l| l.dev_info.devnode.clone())
                            .unwrap_or_else(|| strat_info.dev_info.devnode.clone()),
                        LInfo::Luks(luks_info) => luks_info.dev_info.devnode.clone(),
                    };
                    PoolDevice {
                        devnode,
                        uuid: *uuid,
                    }
                })
                .collect::<Vec<_>>(),
        }
    }

    /// Process the data from a remove udev event. Since remove events are
    /// always subtractive, this method can never introduce a key_description
    /// which is incompatible with the existing key description.
    pub fn process_info_remove(&mut self, path: &Path, pool_uuid: PoolUuid, dev_uuid: DevUuid) {
        match self.internal.remove(&dev_uuid) {
            Some(LInfo::Luks(linfo)) => {
                if path == linfo.dev_info.devnode {
                    info!(
                        "Device with pool UUID {}, device UUID {} is no longer available",
                        pool_uuid, dev_uuid
                    );
                } else {
                    warn!("Device with pool UUID {}, device UUID {} appears to have been removed but the path did not match the known Stratis device with these identifiers", pool_uuid, dev_uuid);
                    self.internal.insert(dev_uuid, LInfo::Luks(linfo));
                }
            }
            Some(LInfo::Stratis(sinfo)) => {
                if Some(path) == sinfo.luks.as_ref().map(|i| i.dev_info.devnode.as_path()) {
                    info!("Encrypted backing device with pool UUID {}, device UUID {} is no longer available; removing activated devicemapper device as well", pool_uuid, dev_uuid);
                } else if path == sinfo.dev_info.devnode {
                    if let Some(l) = sinfo.luks {
                        info!("Encrypted Stratis device with pool UUID {}, device UUID {} is no longer available; marking encrypted backing device as closed", pool_uuid, dev_uuid);
                        self.internal.insert(dev_uuid, LInfo::Luks(l));
                    } else {
                        info!("Stratis device with pool UUID {}, device UUID {} is no longer available", pool_uuid, dev_uuid);
                    }
                }
            }
            _ => (),
        }
    }

    /// Process the data from an add udev event. If the added data is
    /// incompatible with the existing, warn the user and ignore the new
    /// entry.
    pub fn process_info_add(&mut self, info: DeviceInfo) {
        let stratis_identifiers = info.stratis_identifiers();
        let device_uuid = stratis_identifiers.device_uuid;

        match self.internal.remove(&device_uuid) {
            None => {
                info!(
                    "Device information {} discovered and inserted into the set for its pool UUID",
                    info
                );
                self.internal.insert(device_uuid, info.into());
            }
            Some(removed) => match LInfo::update(removed, info) {
                Either::Right(info) => {
                    warn!("Found a device information that conflicts with an existing registered device; ignoring");
                    self.internal.insert(device_uuid, info);
                }
                Either::Left(info) => {
                    info!(
                        "Device information {} replaces previous device information for the same device UUID in the set for its pool UUID",
                        info
                    );
                    self.internal.insert(device_uuid, info);
                }
            },
        }
    }

    /// Returns a boolean indicating whether the data structure has any devices
    /// registered.
    pub fn is_empty(&self) -> bool {
        self.internal.is_empty()
    }
}

impl<'a> Into<Value> for &'a DeviceSet {
    fn into(self) -> Value {
        Value::Array(self.internal.values().map(|info| info.into()).collect())
    }
}

/// A miscellaneous grab bag of devices; there may be devices w/ duplicated
/// UUIDs, for instance.
#[derive(Debug, Eq, PartialEq)]
pub struct DeviceBag {
    internal: HashSet<LInfo>,
}

impl DeviceBag {
    pub fn remove(&mut self, path: &Path, pool_uuid: PoolUuid, dev_uuid: DevUuid) {
        self.internal = self
            .internal
            .drain()
            .filter_map(|i| LInfo::update_on_remove(i, path, pool_uuid, dev_uuid))
            .collect::<HashSet<_>>();
    }

    pub fn insert(&mut self, info: LInfo) -> bool {
        self.internal.insert(info)
    }

    pub fn extend<I: IntoIterator<Item = LInfo>>(&mut self, iter: I) {
        self.internal.extend(iter)
    }
}

impl<'a> Into<Value> for &'a DeviceBag {
    fn into(self) -> Value {
        Value::Array(self.internal.iter().map(|info| info.into()).collect())
    }
}
