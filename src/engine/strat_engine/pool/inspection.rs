// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{hash_map::Entry, HashMap},
    fmt,
};

use devicemapper::Sectors;

use crate::{
    engine::{strat_engine::serde_structs::PoolSave, types::DevUuid},
    stratis::{StratisError, StratisResult},
};

const SIZE_OF_CRYPT_METADATA_SECTORS: Sectors = Sectors(32768);
const SIZE_OF_STRATIS_METADATA_SECTORS: Sectors = Sectors(8192);

// Encodes the use for each extent.
trait Use: Copy + Eq + PartialEq + fmt::Display {}

trait Allocator<U: Use> {
    // How to mark an unused portion of the device.
    fn unused_marker() -> U;

    // Offset from the start of the device to perform calculations.
    fn offset(&self) -> Sectors;

    // The recorded extents of the offset
    fn extents(&self) -> &HashMap<Sectors, (U, Sectors)>;

    // A table using the unused marker for unused extents
    fn filled(&self) -> HashMap<Sectors, (U, Sectors)> {
        filled(self.extents(), Self::unused_marker(), self.offset())
    }

    // The sum of the lengths of all the extents that belong to any of the
    // uses.
    fn sum(&self, uses: &[U]) -> Sectors {
        sum(&self.filled(), uses)
    }
}

// Return the sum of the length of all the extents in extents that fall into
// any of the list of uses. An empty list of uses will always result in a sum
// of 0 sectors.
fn sum<U>(extents: &HashMap<Sectors, (U, Sectors)>, uses: &[U]) -> Sectors
where
    U: Use,
{
    extents
        .values()
        .filter_map(|(u, l)| if uses.contains(u) { Some(l) } else { None })
        .cloned()
        .sum()
}

// Returns a map of start sectors to the extents use and length.
// Marks unused parts with the specified filler use.
// Begins at start_offset which must be at least 0, but may be more.
fn filled<U>(
    extents: &HashMap<Sectors, (U, Sectors)>,
    filler: U,
    start_offset: Sectors,
) -> HashMap<Sectors, (U, Sectors)>
where
    U: Use,
{
    let mut result = HashMap::new();
    let mut current_offset = start_offset;
    let mut starts: Vec<&Sectors> = extents.keys().collect();
    starts.sort();

    for &start in starts {
        let (used, length) = extents[&start];
        if start > current_offset {
            result.insert(current_offset, (filler, start - current_offset));
        }
        result.insert(start, (used, length));
        current_offset = start + length;
    }

    result
}

// Add an optional vector of extents to the current data structure.
fn add<U>(
    current: &mut HashMap<Sectors, (U, Sectors)>,
    to_add: &[(Sectors, Sectors)],
    used: U,
) -> StratisResult<()>
where
    U: Use,
{
    for (start, length) in to_add.iter() {
        if current.contains_key(start) {
            return Err(StratisError::Msg(format!(
                "Key collision: {start} already in extents table"
            )));
        }
        current.insert(*start, (used, *length));
    }

    Ok(())
}

// Print a representation of extents for display.
fn display<U>(f: &mut fmt::Formatter<'_>, extents: &HashMap<Sectors, (U, Sectors)>) -> fmt::Result
where
    U: Use,
{
    let mut starts: Vec<&Sectors> = extents.keys().collect();
    starts.sort();
    for (index, &&start) in starts.iter().enumerate() {
        let (used, length) = extents[&start];
        let end = start + length;
        let (start, length, end) = (*start, *length, *end);
        writeln!(
            f,
            "{index}: Use: {used:20} {start:12} + {length:12} = {end:12} sectors"
        )?;
    }
    Ok(())
}

// Check whether any extent overlaps with another.
fn check_overlap<U>(extents: &HashMap<Sectors, (U, Sectors)>, start_offset: Sectors) -> Vec<String>
where
    U: Use,
{
    let mut errors = vec![];
    let mut current_offset = start_offset;
    let mut starts: Vec<&Sectors> = extents.keys().collect();
    starts.sort();

    for &start in starts {
        let (used, length) = extents[&start];
        if start < current_offset {
            errors.push(format!("allocation ({start}, {length}) for {used} overlaps with previous allocation which extends to {current_offset}"))
        }
        current_offset = start + length;
    }

    errors
}

#[derive(strum_macros::Display)]
#[strum(serialize_all = "snake_case")]
#[derive(Clone, Copy, Eq, PartialEq)]
enum CapDeviceUse {
    Allocated,
    Unused,
}

impl Use for CapDeviceUse {}

struct CapDevice {
    extents: HashMap<Sectors, (CapDeviceUse, Sectors)>,
    encrypted: bool,
}

impl CapDevice {
    fn new(encrypted: bool) -> CapDevice {
        CapDevice {
            extents: HashMap::new(),
            encrypted,
        }
    }

    fn add(mut self, allocs: Option<&[(Sectors, Sectors)]>) -> StratisResult<Self> {
        if let Some(allocs) = allocs {
            add(&mut self.extents, allocs, CapDeviceUse::Allocated)?;
        }

        Ok(self)
    }

    fn check(&self) -> Vec<String> {
        let mut errors = Vec::new();
        errors.extend(check_overlap(&self.extents, self.offset()));
        errors
    }
}

impl Allocator<CapDeviceUse> for CapDevice {
    fn offset(&self) -> Sectors {
        if self.encrypted {
            Sectors(0)
        } else {
            SIZE_OF_CRYPT_METADATA_SECTORS
        }
    }

    fn unused_marker() -> CapDeviceUse {
        CapDeviceUse::Unused
    }

    fn extents(&self) -> &HashMap<Sectors, (CapDeviceUse, Sectors)> {
        &self.extents
    }
}

impl fmt::Display for CapDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        display(f, &self.filled())
    }
}

#[derive(strum_macros::Display)]
#[strum(serialize_all = "snake_case")]
#[derive(Clone, Copy, Eq, PartialEq)]
enum DataDeviceUse {
    StratisMetadata,
    IntegrityMetadata,
    Allocated,
    Unused,
}

impl Use for DataDeviceUse {}

struct DataDevice {
    extents: HashMap<Sectors, (DataDeviceUse, Sectors)>,
}

impl DataDevice {
    fn new() -> DataDevice {
        let mut extents = HashMap::new();
        extents.insert(
            Sectors(0),
            (
                DataDeviceUse::StratisMetadata,
                SIZE_OF_STRATIS_METADATA_SECTORS,
            ),
        );
        DataDevice { extents }
    }

    fn add(
        mut self,
        integrity_meta_allocs: Option<&Vec<(Sectors, Sectors)>>,
        allocs: Option<&[(Sectors, Sectors)]>,
    ) -> StratisResult<Self> {
        if let Some(allocs) = integrity_meta_allocs {
            add(&mut self.extents, allocs, DataDeviceUse::IntegrityMetadata)?;
        }

        if let Some(allocs) = allocs {
            add(&mut self.extents, allocs, DataDeviceUse::Allocated)?;
        }

        Ok(self)
    }

    fn _check_integrity_meta_round(&self) -> Vec<String> {
        let mut errors = Vec::new();

        for (_, &(_, length)) in self
            .extents
            .iter()
            .filter(|(_, &(used, _))| used == DataDeviceUse::IntegrityMetadata)
        {
            if length % Sectors(8) != Sectors(0) {
                errors.push(format!(
                    "Allocation {length} for integrity meta data not a multiple of 4KiB"
                ));
            }
        }

        errors
    }

    fn check(&self) -> Vec<String> {
        let mut errors = Vec::new();
        errors.extend(check_overlap(&self.extents, self.offset()));
        errors.extend(self._check_integrity_meta_round());
        errors
    }
}

impl Allocator<DataDeviceUse> for DataDevice {
    #[allow(clippy::unused_self)]
    fn offset(&self) -> Sectors {
        Sectors(0)
    }

    fn unused_marker() -> DataDeviceUse {
        DataDeviceUse::Unused
    }

    fn extents(&self) -> &HashMap<Sectors, (DataDeviceUse, Sectors)> {
        &self.extents
    }
}

impl fmt::Display for DataDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        display(f, &self.filled())
    }
}

#[derive(strum_macros::Display)]
#[strum(serialize_all = "snake_case")]
#[derive(Clone, Copy, Eq, PartialEq)]
enum CacheDeviceUse {
    StratisMetadata,
    CacheMetadata,
    CacheData,
    Unused,
}

impl Use for CacheDeviceUse {}

struct CacheDevice {
    extents: HashMap<Sectors, (CacheDeviceUse, Sectors)>,
}

impl CacheDevice {
    fn new() -> CacheDevice {
        let mut extents = HashMap::new();
        extents.insert(
            Sectors(0),
            (
                CacheDeviceUse::StratisMetadata,
                SIZE_OF_STRATIS_METADATA_SECTORS,
            ),
        );
        CacheDevice { extents }
    }

    fn add(
        mut self,
        metadata_allocs: Option<&[(Sectors, Sectors)]>,
        data_allocs: Option<&[(Sectors, Sectors)]>,
    ) -> StratisResult<Self> {
        if let Some(allocs) = metadata_allocs {
            add(&mut self.extents, allocs, CacheDeviceUse::CacheMetadata)?;
        }

        if let Some(allocs) = data_allocs {
            add(&mut self.extents, allocs, CacheDeviceUse::CacheData)?;
        }

        Ok(self)
    }

    fn check(&self) -> Vec<String> {
        let mut errors = Vec::new();
        errors.extend(check_overlap(&self.extents, self.offset()));
        errors
    }
}

impl Allocator<CacheDeviceUse> for CacheDevice {
    #[allow(clippy::unused_self)]
    fn offset(&self) -> Sectors {
        Sectors(0)
    }

    fn unused_marker() -> CacheDeviceUse {
        CacheDeviceUse::Unused
    }

    fn extents(&self) -> &HashMap<Sectors, (CacheDeviceUse, Sectors)> {
        &self.extents
    }
}

impl fmt::Display for CacheDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        display(f, &self.filled())
    }
}

#[derive(strum_macros::Display)]
#[strum(serialize_all = "snake_case")]
#[derive(Clone, Copy, Eq, PartialEq)]
enum CryptAllocsUse {
    Metadata,
}

impl Use for CryptAllocsUse {}

struct CryptAllocs {
    extents: HashMap<Sectors, (CryptAllocsUse, Sectors)>,
}

impl CryptAllocs {
    fn new() -> CryptAllocs {
        CryptAllocs {
            extents: HashMap::new(),
        }
    }

    fn add(mut self, allocs: Option<&Vec<(Sectors, Sectors)>>) -> StratisResult<Self> {
        if let Some(allocs) = allocs {
            add(&mut self.extents, allocs, CryptAllocsUse::Metadata)?;
        }

        Ok(self)
    }

    fn check(&self) -> Vec<String> {
        let mut errors = vec![];

        if self.extents.is_empty() {
            errors.push("No allocations for crypt metadata".into());
        }

        if self.extents.len() > 1 {
            errors.push("Multiple allocations for crypt metadata".into());
        }

        let (&start, &(_, length)) = self
            .extents
            .iter()
            .collect::<Vec<_>>()
            .pop()
            .expect("Exactly one extents in the extent map");

        if start != Sectors(0) {
            errors.push(format!("Crypt meta allocs offset, {start} is not 0"));
        }

        if length != SIZE_OF_CRYPT_METADATA_SECTORS {
            errors.push(format!(
                "Crypt meta allocs entry has unexpected length {length}"
            ));
        }

        errors
    }
}

impl fmt::Display for CryptAllocs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        display(f, &self.extents)
    }
}

#[derive(strum_macros::Display)]
#[strum(serialize_all = "snake_case")]
#[derive(Clone, Copy, Eq, PartialEq)]
enum FlexDeviceUse {
    MetaDev,
    ThinDataDev,
    ThinMetaDev,
    ThinMetaDevSpare,
    Unused,
}

impl Use for FlexDeviceUse {}

struct FlexDevice {
    extents: HashMap<Sectors, (FlexDeviceUse, Sectors)>,
    encrypted: bool,
}

impl FlexDevice {
    fn new(encrypted: bool) -> FlexDevice {
        FlexDevice {
            extents: HashMap::new(),
            encrypted,
        }
    }

    fn add(
        mut self,
        thin_meta_dev: Option<&Vec<(Sectors, Sectors)>>,
        thin_meta_dev_spare: Option<&Vec<(Sectors, Sectors)>>,
        meta_dev: Option<&Vec<(Sectors, Sectors)>>,
        thin_data_dev: Option<&Vec<(Sectors, Sectors)>>,
    ) -> StratisResult<Self> {
        if let Some(allocs) = thin_meta_dev {
            add(&mut self.extents, allocs, FlexDeviceUse::ThinMetaDev)?;
        }

        if let Some(allocs) = thin_meta_dev_spare {
            add(&mut self.extents, allocs, FlexDeviceUse::ThinMetaDevSpare)?;
        }

        if let Some(allocs) = meta_dev {
            add(&mut self.extents, allocs, FlexDeviceUse::MetaDev)?;
        }

        if let Some(allocs) = thin_data_dev {
            add(&mut self.extents, allocs, FlexDeviceUse::ThinDataDev)?;
        }

        Ok(self)
    }

    // Verify that both thin meta devices, the one currently in use and the
    // spare, are the same size.
    fn _check_thin_metas_equal(&self) -> Vec<String> {
        let thin_meta_total = self.sum(&[FlexDeviceUse::ThinMetaDev]);
        let thin_meta_spare_total = self.sum(&[FlexDeviceUse::ThinMetaDevSpare]);
        if thin_meta_total == thin_meta_spare_total {
            vec![]
        } else {
            vec![format!("The sum of the allocations for the thin meta device, {thin_meta_total}, does not equal the sum of the allocations for the thin meta spare device, {thin_meta_spare_total}.")]
        }
    }

    fn check(&self) -> Vec<String> {
        let mut errors = Vec::new();
        errors.extend(self._check_thin_metas_equal());
        errors.extend(check_overlap(&self.extents, self.offset()));
        errors
    }
}

impl Allocator<FlexDeviceUse> for FlexDevice {
    fn offset(&self) -> Sectors {
        if self.encrypted {
            Sectors(0)
        } else {
            SIZE_OF_CRYPT_METADATA_SECTORS
        }
    }

    fn unused_marker() -> FlexDeviceUse {
        FlexDeviceUse::Unused
    }

    fn extents(&self) -> &HashMap<Sectors, (FlexDeviceUse, Sectors)> {
        &self.extents
    }
}

impl fmt::Display for FlexDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        display(f, &self.filled())
    }
}

// Calculate map of device UUIDs to data device representation from metadata.
fn data_devices(metadata: &PoolSave) -> StratisResult<HashMap<DevUuid, DataDevice>> {
    let data_tier_metadata = &metadata.backstore.data_tier;

    let data_tier_devs = &data_tier_metadata.blockdev.devs;

    let mut bds = data_tier_devs
        .iter()
        .try_fold(HashMap::new(), |mut acc, dev| {
            if let Entry::Vacant(e) = acc.entry(dev.uuid) {
                e.insert(DataDevice::new().add(Some(&dev.integrity_meta_allocs), None)?);
                Ok(acc)
            } else {
                Err(StratisError::Msg(format!(
                    "Two devices with same UUID {} in devs structure",
                    dev.uuid
                )))
            }
        })?;

    let data_tier_allocs = &data_tier_metadata.blockdev.allocs[0];

    for item in data_tier_allocs {
        if let Some(dd) = bds.remove(&item.parent) {
            bds.insert(
                item.parent,
                dd.add(None, Some(&[(item.start, item.length)]))?,
            );
        } else {
            return Err(StratisError::Msg(format!(
                "No device in devs for uuid {} in blockdevs",
                item.parent
            )));
        }
    }

    Ok(bds)
}

// Calculate map of device UUIDs to cache device representation from metadata.
fn cache_devices(metadata: &PoolSave) -> StratisResult<HashMap<DevUuid, CacheDevice>> {
    let cache_tier_metadata = &metadata.backstore.cache_tier;

    cache_tier_metadata.as_ref().map_or_else(
        || Ok(HashMap::new()),
        |cache_tier_metadata| {
            let cache_tier_devs = &cache_tier_metadata.blockdev.devs;
            let mut bds = cache_tier_devs
                .iter()
                .try_fold(HashMap::new(), |mut acc, dev| {
                    if let Entry::Vacant(e) = acc.entry(dev.uuid) {
                        e.insert(CacheDevice::new());
                        Ok(acc)
                    } else {
                        Err(StratisError::Msg(format!(
                            "Two devices with same UUID {} in devs structure",
                            dev.uuid
                        )))
                    }
                })?;

            let cache_tier_allocs = &cache_tier_metadata.blockdev.allocs;

            for item in &cache_tier_allocs[0] {
                if let Some(dd) = bds.remove(&item.parent) {
                    bds.insert(
                        item.parent,
                        dd.add(Some(&[(item.start, item.length)]), None)?,
                    );
                } else {
                    return Err(StratisError::Msg(format!(
                        "No device in devs for uuid {} in blockdevs",
                        item.parent
                    )));
                }
            }

            for item in &cache_tier_allocs[1] {
                if let Some(dd) = bds.remove(&item.parent) {
                    bds.insert(
                        item.parent,
                        dd.add(None, Some(&[(item.start, item.length)]))?,
                    );
                } else {
                    return Err(StratisError::Msg(format!(
                        "No device in devs for uuid {} in blockdevs",
                        item.parent
                    )));
                }
            }

            Ok(bds)
        },
    )
}

// Calculate allocations for crypt metadata from the metadata.
fn crypt_allocs(metadata: &PoolSave) -> StratisResult<CryptAllocs> {
    let crypt_metadata = &metadata.backstore.cap.crypt_meta_allocs;

    CryptAllocs::new().add(Some(crypt_metadata))
}

// Calculate the flex device from the metadata.
fn flex_device(metadata: &PoolSave, encrypted: bool) -> StratisResult<FlexDevice> {
    let flex_device_metadata = &metadata.flex_devs;

    FlexDevice::new(encrypted).add(
        Some(&flex_device_metadata.thin_meta_dev),
        Some(&flex_device_metadata.thin_meta_dev_spare),
        Some(&flex_device_metadata.meta_dev),
        Some(&flex_device_metadata.thin_data_dev),
    )
}

fn cap_device(metadata: &PoolSave, encrypted: bool) -> StratisResult<CapDevice> {
    let cap_device_metadata = &metadata.backstore.cap;

    CapDevice::new(encrypted).add(Some(&cap_device_metadata.allocs))
}

/// Some ways of inspecting the pool-level metadata.
pub mod inspectors {
    use super::{
        cache_devices, cap_device, crypt_allocs, data_devices, flex_device, PoolSave, StratisResult,
    };

    use crate::{engine::strat_engine::serde_structs::PoolFeatures, stratis::StratisError};

    /// Check that the metadata is well-formed.
    pub fn check(metadata: &PoolSave) -> StratisResult<()> {
        let mut errors = Vec::new();

        let encrypted = metadata.features.contains(&PoolFeatures::Encryption);

        let data_devices = data_devices(metadata)?;
        for data_device in data_devices.values() {
            errors.extend(data_device.check());
        }

        let cache_devices = cache_devices(metadata)?;
        for cache_device in cache_devices.values() {
            errors.extend(cache_device.check());
        }

        let crypt_allocs = crypt_allocs(metadata)?;
        errors.extend(crypt_allocs.check());

        let cap_device = cap_device(metadata, encrypted)?;
        errors.extend(cap_device.check());

        let flex_device = flex_device(metadata, encrypted)?;
        errors.extend(flex_device.check());

        if !errors.is_empty() {
            Err(StratisError::Msg(errors.join("\n")))
        } else {
            Ok(())
        }
    }

    /// Print a human-useful representation of the metadata's meaning.
    pub fn print(metadata: &PoolSave) -> StratisResult<()> {
        let encrypted = metadata.features.contains(&PoolFeatures::Encryption);

        let crypt_allocs = crypt_allocs(metadata)?;
        let flex_device = flex_device(metadata, encrypted)?;
        let data_devices = data_devices(metadata)?;
        let cache_devices = cache_devices(metadata)?;
        let cap_device = cap_device(metadata, encrypted)?;

        println!("Allocations from each data device:");
        for (uuid, bd) in data_devices.iter() {
            println!("Data device: {uuid}");
            println!("{}", bd);
        }

        println!();

        println!("Allocations from each cache device:");
        for (uuid, bd) in cache_devices.iter() {
            println!("Cache device: {uuid}");
            println!("{}", bd);
        }

        println!();

        println!("Allocations for crypt metadata:");
        print!("{}", crypt_allocs);

        println!();

        println!("Allocations from cap device:");
        println!("{}", cap_device);

        println!();

        println!("Allocations from flex device:");
        print!("{}", flex_device);
        Ok(())
    }
}