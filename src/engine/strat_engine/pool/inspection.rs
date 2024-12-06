// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{cmp::max, collections::HashMap, fmt};

use devicemapper::Sectors;

use crate::{
    engine::strat_engine::serde_structs::PoolSave,
    stratis::{StratisError, StratisResult},
};

const SIZE_OF_CRYPT_METADATA_SECTORS: Sectors = Sectors(32768);

// Returns a map of start sectors to the extents use and length.
// Marks unused parts with the specified filler use.
// Begins at start_offset which must be at least 0, but may be more.
fn filled<U>(
    extents: &HashMap<Sectors, (U, Sectors)>,
    filler: U,
    start_offset: Sectors,
) -> HashMap<Sectors, (U, Sectors)>
where
    U: Copy,
{
    let mut result = HashMap::new();
    let mut current_offset = start_offset;
    let mut starts: Vec<&Sectors> = extents.keys().collect();
    starts.sort();

    for &start in starts {
        let (used, length) = extents[&start];
        if start > current_offset {
            result.insert(start, (filler, start - current_offset));
        }
        result.insert(start, (used, length));
        current_offset = start + length;
    }

    result
}

// Find the exclusive endpoint of the last extent or start_offset,
// whichever is greater.
fn max_extent<U>(extents: &HashMap<Sectors, (U, Sectors)>, start_offset: Sectors) -> Sectors {
    extents
        .iter()
        .map(|(&start, &(_, length))| start + length)
        .max()
        .map(|res| max(res, start_offset))
        .unwrap_or(start_offset)
}

// Check whether any extents overlap with each other.
fn check_overlap<U>(extents: &HashMap<Sectors, (U, Sectors)>, start_offset: Sectors) -> Vec<String>
where
    U: Copy + fmt::Display,
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
enum CryptAllocsUse {
    Metadata,
}

struct CryptAllocs {
    extents: HashMap<Sectors, (CryptAllocsUse, Sectors)>,
}

impl CryptAllocs {
    fn new() -> CryptAllocs {
        CryptAllocs {
            extents: HashMap::new(),
        }
    }

    fn add(&mut self, allocs: Option<&Vec<(Sectors, Sectors)>>) -> StratisResult<()> {
        if let Some(extents) = allocs {
            for (start, length) in extents.iter() {
                if self.extents.contains_key(start) {
                    return Err(StratisError::Msg(format!(
                        "Key collision: {start} already in extents table"
                    )));
                }
                self.extents
                    .insert(*start, (CryptAllocsUse::Metadata, *length));
            }
        }

        Ok(())
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
        let mut starts: Vec<&Sectors> = self.extents.keys().collect();
        starts.sort();
        for (index, &&start) in starts.iter().enumerate() {
            let (used, length) = self.extents[&start];
            let end = start + length;
            let (start, length, end) = (*start, *length, *end);
            writeln!(
                f,
                "{index}: Use: {used:20} {start:12} + {length:12} = {end:12} sectors"
            )?;
        }
        Ok(())
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
        &mut self,
        thin_meta_dev: Option<&Vec<(Sectors, Sectors)>>,
        thin_meta_dev_spare: Option<&Vec<(Sectors, Sectors)>>,
        meta_dev: Option<&Vec<(Sectors, Sectors)>>,
        thin_data_dev: Option<&Vec<(Sectors, Sectors)>>,
    ) -> StratisResult<()> {
        if let Some(extents) = thin_meta_dev {
            for (start, length) in extents.iter() {
                if self.extents.contains_key(start) {
                    return Err(StratisError::Msg(format!(
                        "Key collision: {start} already in extents table"
                    )));
                }
                self.extents
                    .insert(*start, (FlexDeviceUse::ThinMetaDev, *length));
            }
        }

        if let Some(extents) = thin_meta_dev_spare {
            for (start, length) in extents.iter() {
                if self.extents.contains_key(start) {
                    return Err(StratisError::Msg(format!(
                        "Key collision: {start} already in extents table"
                    )));
                }
                self.extents
                    .insert(*start, (FlexDeviceUse::ThinMetaDevSpare, *length));
            }
        }

        if let Some(extents) = meta_dev {
            for (start, length) in extents.iter() {
                if self.extents.contains_key(start) {
                    return Err(StratisError::Msg(format!(
                        "Key collision: {start} already in extents table"
                    )));
                }
                self.extents
                    .insert(*start, (FlexDeviceUse::MetaDev, *length));
            }
        }

        if let Some(extents) = thin_data_dev {
            for (start, length) in extents.iter() {
                if self.extents.contains_key(start) {
                    return Err(StratisError::Msg(format!(
                        "Key collision: {start} already in extents table"
                    )));
                }
                self.extents
                    .insert(*start, (FlexDeviceUse::ThinDataDev, *length));
            }
        }

        Ok(())
    }

    // Offset from start of devices where allocations from the device begin.
    fn offset(&self) -> Sectors {
        if self.encrypted {
            Sectors(0)
        } else {
            SIZE_OF_CRYPT_METADATA_SECTORS
        }
    }

    fn filled(&self) -> HashMap<Sectors, (FlexDeviceUse, Sectors)> {
        filled(&self.extents, FlexDeviceUse::Unused, self.offset())
    }

    #[allow(dead_code)]
    fn max_extent(&self) -> Sectors {
        max_extent(&self.extents, self.offset())
    }

    // sum of the lengths devoted to a particular set of uses.
    // If uses is None, some all the extents.
    fn sum(&self, uses: Option<&[FlexDeviceUse]>) -> Sectors {
        self.filled()
            .values()
            .filter_map(|(u, l)| {
                if uses.map(|uses| uses.contains(u)).unwrap_or(true) {
                    Some(l)
                } else {
                    None
                }
            })
            .cloned()
            .sum()
    }

    // Verify that both thin meta devices, the one currently in use and the
    // spare, are the same size.
    fn _check_thin_metas_equal(&self) -> Vec<String> {
        let thin_meta_total = self.sum(Some(&[FlexDeviceUse::ThinMetaDev]));
        let thin_meta_spare_total = self.sum(Some(&[FlexDeviceUse::ThinMetaDevSpare]));
        if thin_meta_total == thin_meta_spare_total {
            vec![]
        } else {
            vec![format!("The sum of the allocations for the thin meta device, {thin_meta_total}, does not equal the sum of the allocations for the thin meta spare device, {thin_meta_spare_total}.")]
        }
    }

    // Check some properties of the device.
    fn check(&self) -> Vec<String> {
        let mut errors = Vec::new();
        errors.extend(self._check_thin_metas_equal());
        errors.extend(check_overlap(&self.extents, self.offset()));
        errors
    }
}

impl fmt::Display for FlexDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let table = self.filled();
        let mut starts: Vec<&Sectors> = table.keys().collect();
        starts.sort();
        for (index, &&start) in starts.iter().enumerate() {
            let (used, length) = table[&start];
            let end = start + length;
            let (start, length, end) = (*start, *length, *end);
            writeln!(
                f,
                "{index}: Use: {used:20} {start:12} + {length:12} = {end:12} sectors"
            )?;
        }
        Ok(())
    }
}

fn crypt_allocs(metadata: &PoolSave) -> StratisResult<CryptAllocs> {
    let mut crypt_allocs = CryptAllocs::new();
    let crypt_metadata = &metadata.backstore.cap.crypt_meta_allocs;

    crypt_allocs.add(Some(crypt_metadata))?;

    Ok(crypt_allocs)
}

// Calculate the flex device from the metadata.
fn flex_device(metadata: &PoolSave, encrypted: bool) -> StratisResult<FlexDevice> {
    let mut flex_device = FlexDevice::new(encrypted);
    let flex_device_metadata = &metadata.flex_devs;

    flex_device.add(
        Some(&flex_device_metadata.thin_meta_dev),
        Some(&flex_device_metadata.thin_meta_dev_spare),
        Some(&flex_device_metadata.meta_dev),
        Some(&flex_device_metadata.thin_data_dev),
    )?;

    Ok(flex_device)
}

/// Some ways of inspecting the pool-level metadata.
pub mod inspectors {
    use super::{crypt_allocs, flex_device, PoolSave, StratisResult};

    use crate::stratis::StratisError;

    /// Check that the metadata is well-formed.
    pub fn check(metadata: &PoolSave) -> StratisResult<()> {
        let mut errors = Vec::new();
        let crypt_allocs = crypt_allocs(metadata)?;
        errors.extend(crypt_allocs.check());

        let flex_device = flex_device(metadata, false)?;
        errors.extend(flex_device.check());

        if !errors.is_empty() {
            Err(StratisError::Msg(errors.join("\n")))
        } else {
            Ok(())
        }
    }

    /// Print a human-useful representation of the metadata's meaning.
    pub fn print(metadata: &PoolSave) -> StratisResult<()> {
        let crypt_allocs = crypt_allocs(metadata)?;
        let flex_device = flex_device(metadata, false)?;

        println!("Allocations for crypt metadata:");
        print!("{}", crypt_allocs);

        println!();
        println!("Allocations from flex device:");
        print!("{}", flex_device);
        Ok(())
    }
}
