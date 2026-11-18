// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{iter::IntoIterator, path::Path};

use crate::{
    engine::{
        shared::validate_name,
        strat_engine::{
            backstore::{ProcessedPathInfos, StratisDevices, UnownedDevices},
            keys::validate_key_descs,
            serde_structs::FilesystemSave,
        },
        types::{InputEncryptionInfo, IntegritySpec, Name, ValidatedIntegritySpec},
    },
    stratis::StratisResult,
};

/// Define how an origin and its snapshot are merged when a filesystem is
/// reverted.
pub fn merge(origin: &FilesystemSave, snap: &FilesystemSave) -> FilesystemSave {
    FilesystemSave {
        name: origin.name.to_owned(),
        uuid: origin.uuid,
        thin_id: snap.thin_id,
        size: snap.size,
        created: origin.created,
        fs_size_limit: snap.fs_size_limit,
        origin: origin.origin,
        merge: origin.merge,
    }
}

pub fn shift_allocation_offset<'a, T: 'a>(
    iter: impl IntoIterator<Item = &'a T>,
    offset_map: impl Fn(&'a T) -> StratisResult<T>,
) -> StratisResult<Vec<T>> {
    iter.into_iter()
        .map(offset_map)
        .collect::<StratisResult<Vec<T>>>()
}

pub async fn validate_input_v1(
    encryption_info: Option<&InputEncryptionInfo>,
    name: &str,
    blockdev_paths: &[&Path],
) -> StratisResult<(Name, StratisDevices, UnownedDevices)> {
    if let Some(ei) = encryption_info {
        validate_key_descs(ei.key_descs())?;
    }

    validate_name(name)?;
    let name = Name::new(name.to_owned());

    let cloned_paths = blockdev_paths
        .iter()
        .map(|p| p.to_path_buf())
        .collect::<Vec<_>>();

    let devices = spawn_blocking!({
        let borrowed_paths = cloned_paths.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        ProcessedPathInfos::try_from(borrowed_paths.as_slice())
    })??;
    let (stratis_devices, unowned_devices) = devices.unpack();

    Ok((name, stratis_devices, unowned_devices))
}

pub async fn validate_input_v2(
    encryption_info: Option<&InputEncryptionInfo>,
    name: &str,
    blockdev_paths: &[&Path],
    integrity_spec: IntegritySpec,
) -> StratisResult<(Name, StratisDevices, UnownedDevices, ValidatedIntegritySpec)> {
    let (name, stratis_devices, unowned_devices) =
        validate_input_v1(encryption_info, name, blockdev_paths).await?;
    let int_spec = ValidatedIntegritySpec::try_from(integrity_spec)?;

    Ok((name, stratis_devices, unowned_devices, int_spec))
}
