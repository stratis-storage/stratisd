// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Get ability to instantiate a devicemapper context.

use std::{path::Path, sync::LazyLock};

use devicemapper::{DevId, DmError, DmNameBuf, DmOptions, DmUuidBuf, DM};

use crate::{
    engine::{
        strat_engine::names::{
            format_backstore_ids, format_backstore_migrate_name, format_crypt_backstore_name,
            format_crypt_name, format_flex_ids, format_thin_ids, format_thinpool_ids, CacheRole,
            FlexRole, ThinPoolRole, ThinRole,
        },
        types::{DevUuid, FilesystemUuid, PoolUuid},
    },
    stratis::{StratisError, StratisResult},
};

/// Path to logical devices for encrypted devices
pub const DEVICEMAPPER_PATH: &str = "/dev/mapper";

/// The roles of all DM devices that may make up a backstore, ordered so that
/// every device precedes the devices it is built on top of. This is the order
/// in which they must be removed.
const BACKSTORE_ROLES: [CacheRole; 4] = [
    CacheRole::Cache,
    CacheRole::CacheSub,
    CacheRole::MetaSub,
    CacheRole::OriginSub,
];

pub static DM_CONTEXT: LazyLock<Result<DM, DmError>> = LazyLock::new(DM::new);

pub fn get_dm_init() -> StratisResult<&'static DM> {
    match *DM_CONTEXT {
        Ok(ref context) => Ok(context),
        Err(ref e) => Err(StratisError::Chained(
            "Failed to initialize DM context".to_string(),
            Box::new(e.clone().into()),
        )),
    }
}

pub fn get_dm() -> &'static DM {
    get_dm_init().expect(
        "the engine has already called get_dm_init() and exited if get_dm_init() returned an error",
    )
}

pub fn remove_optional_devices(devs: Vec<DmNameBuf>) -> StratisResult<()> {
    let devices = get_dm()
        .list_devices()?
        .into_iter()
        .map(|(name, _, _)| name)
        .collect::<Vec<_>>();
    for device in devs {
        if devices.contains(&device) {
            get_dm().device_remove(&DevId::Name(&device), DmOptions::default())?;
        }
    }
    Ok(())
}

/// Rename each of the given DM devices from the first name in the pair to the
/// second. Devices that do not exist are skipped. If any rename fails, the
/// renames that have already succeeded are undone before returning the error,
/// so that on failure no device has been left under a new name.
pub fn rename_optional_devices(renames: &[(DmNameBuf, DmNameBuf)]) -> StratisResult<()> {
    let devices = get_dm()
        .list_devices()?
        .into_iter()
        .map(|(name, _, _)| name)
        .collect::<Vec<_>>();
    for (index, (old_name, new_name)) in renames.iter().enumerate() {
        if !devices.contains(old_name) {
            continue;
        }
        if let Err(e) = get_dm().device_rename(old_name, &DevId::Name(new_name)) {
            for (old_name, new_name) in renames[..index].iter() {
                if devices.contains(old_name) {
                    if let Err(e) = get_dm().device_rename(new_name, &DevId::Name(old_name)) {
                        warn!(
                            "Failed to restore the name of devicemapper device {} to {}: {e}; it may need to be manually renamed",
                            &**new_name, &**old_name
                        );
                    }
                }
            }
            return Err(StratisError::from(e));
        }
    }
    Ok(())
}

/// Set the UUID of each of the given DM devices. Devices that do not exist and
/// devices that already have a UUID are skipped; the kernel permits a DM
/// device's UUID to be set only once, and only if it was not set at creation.
pub fn set_optional_device_uuids(uuids: &[(DmNameBuf, DmUuidBuf)]) -> StratisResult<()> {
    let devices = get_dm()
        .list_devices()?
        .into_iter()
        .map(|(name, _, _)| name)
        .collect::<Vec<_>>();
    for (name, uuid) in uuids {
        if !devices.contains(name) {
            continue;
        }
        if get_dm().device_info(&DevId::Name(name))?.uuid().is_some() {
            continue;
        }
        get_dm().device_rename(name, &DevId::Uuid(uuid))?;
    }
    Ok(())
}

pub fn stop_partially_constructed_pool_legacy(
    pool_uuid: PoolUuid,
    dev_uuids: &[DevUuid],
) -> StratisResult<()> {
    let devs = list_of_partial_pool_devices_legacy(pool_uuid, dev_uuids);
    remove_optional_devices(devs)
}

pub fn stop_partially_constructed_pool(pool_uuid: PoolUuid) -> StratisResult<()> {
    let devs = list_of_partial_pool_devices(pool_uuid);
    remove_optional_devices(devs)
}

pub fn thin_device(pool_uuid: PoolUuid, fs_uuid: FilesystemUuid) -> DmNameBuf {
    let (dm_name, _) = format_thin_ids(pool_uuid, ThinRole::Filesystem(fs_uuid));
    dm_name
}

pub fn list_of_thin_pool_devices(pool_uuid: PoolUuid) -> Vec<DmNameBuf> {
    let mut devs = Vec::new();

    let (thin_pool, _) = format_thinpool_ids(pool_uuid, ThinPoolRole::Pool);
    devs.push(thin_pool);
    let (thin_data, _) = format_flex_ids(pool_uuid, FlexRole::ThinData);
    devs.push(thin_data);
    let (thin_meta, _) = format_flex_ids(pool_uuid, FlexRole::ThinMeta);
    devs.push(thin_meta);
    let (thin_meta_spare, _) = format_flex_ids(pool_uuid, FlexRole::ThinMetaSpare);
    devs.push(thin_meta_spare);

    devs
}

pub fn mdv_device(pool_uuid: PoolUuid) -> DmNameBuf {
    let (thin_mdv, _) = format_flex_ids(pool_uuid, FlexRole::MetadataVolume);
    thin_mdv
}

pub fn list_of_backstore_devices_legacy(pool_uuid: PoolUuid) -> Vec<DmNameBuf> {
    BACKSTORE_ROLES
        .iter()
        .map(|role| format_backstore_ids(pool_uuid, *role).0)
        .collect()
}

/// The temporary names of the backstore devices of a pool that is the source
/// of an in-progress migration. These devices have been moved out of the way
/// by `rename_backstore_devices_migrate()` so that the destination
/// backstore can take the canonical names.
pub fn list_of_backstore_devices_migrate(pool_uuid: PoolUuid) -> Vec<DmNameBuf> {
    BACKSTORE_ROLES
        .iter()
        .map(|role| format_backstore_migrate_name(pool_uuid, *role))
        .collect()
}

/// Move the backstore devices of the migration source pool to their temporary
/// names, freeing up the canonical names for the destination backstore.
pub fn rename_backstore_devices_migrate(pool_uuid: PoolUuid) -> StratisResult<()> {
    rename_optional_devices(&backstore_migrate_renames(pool_uuid))
}

/// Move the backstore devices of the migration source pool back to their
/// canonical names. Only valid if the destination backstore has not yet taken
/// those names, that is, if the migration failed before any destination
/// backstore device was created.
pub fn restore_backstore_devices_migrate(pool_uuid: PoolUuid) -> StratisResult<()> {
    rename_optional_devices(
        &backstore_migrate_renames(pool_uuid)
            .into_iter()
            .map(|(canonical, migrate)| (migrate, canonical))
            .collect::<Vec<_>>(),
    )
}

/// Set the UUIDs of the backstore devices of the migration destination pool.
/// These devices were created without UUIDs because the source backstore's
/// devices still held them; this must only be called once the source
/// backstore has been torn down.
pub fn set_backstore_device_uuids(pool_uuid: PoolUuid) -> StratisResult<()> {
    set_optional_device_uuids(
        &BACKSTORE_ROLES
            .iter()
            .map(|role| format_backstore_ids(pool_uuid, *role))
            .collect::<Vec<_>>(),
    )
}

/// Pairs of canonical and temporary backstore device names for a pool, in
/// migration order.
fn backstore_migrate_renames(pool_uuid: PoolUuid) -> Vec<(DmNameBuf, DmNameBuf)> {
    BACKSTORE_ROLES
        .iter()
        .map(|role| {
            (
                format_backstore_ids(pool_uuid, *role).0,
                format_backstore_migrate_name(pool_uuid, *role),
            )
        })
        .collect()
}

pub fn list_of_backstore_devices(pool_uuid: PoolUuid) -> Vec<DmNameBuf> {
    let mut devs = Vec::new();

    let crypt = format_crypt_backstore_name(&pool_uuid);
    devs.push(crypt);

    devs.extend(list_of_backstore_devices_legacy(pool_uuid));

    devs
}

pub fn list_of_crypt_devices(dev_uuids: &[DevUuid]) -> Vec<DmNameBuf> {
    let mut devs = Vec::new();

    for dev_uuid in dev_uuids.iter() {
        let crypt = format_crypt_name(dev_uuid);
        devs.push(crypt);
    }

    devs
}

/// List of device names for removal on partially constructed pool stop. Does not have
/// filesystem names because partially constructed pools are guaranteed not to have any
/// active filesystems.
fn list_of_partial_pool_devices_legacy(
    pool_uuid: PoolUuid,
    dev_uuids: &[DevUuid],
) -> Vec<DmNameBuf> {
    let mut devs = Vec::new();

    devs.extend(list_of_thin_pool_devices(pool_uuid));

    devs.push(mdv_device(pool_uuid));

    devs.extend(list_of_backstore_devices_legacy(pool_uuid));

    devs.extend(list_of_crypt_devices(dev_uuids));

    devs
}

/// List of device names for removal on partially constructed pool stop. Does not have
/// filesystem names because partially constructed pools are guaranteed not to have any
/// active filesystems.
fn list_of_partial_pool_devices(pool_uuid: PoolUuid) -> Vec<DmNameBuf> {
    let mut devs = Vec::new();

    devs.extend(list_of_thin_pool_devices(pool_uuid));

    devs.push(mdv_device(pool_uuid));

    devs.extend(list_of_backstore_devices(pool_uuid));

    devs
}

/// Check whether there are any leftover devicemapper devices from the pool.
pub fn has_leftover_devices_legacy(pool_uuid: PoolUuid, dev_uuids: &[DevUuid]) -> bool {
    let mut has_leftover = false;
    let devices = list_of_partial_pool_devices_legacy(pool_uuid, dev_uuids);
    match get_dm().list_devices() {
        Ok(l) => {
            let listed_devices = l
                .into_iter()
                .map(|(dm_name, _, _)| dm_name)
                .collect::<Vec<_>>();
            for device in devices {
                if listed_devices.contains(&device) {
                    has_leftover |= true;
                }
            }
        }
        Err(_) => {
            for device in devices {
                if Path::new(&format!("/dev/mapper/{}", &*device)).exists() {
                    has_leftover |= true;
                }
            }
        }
    }

    has_leftover
}

/// Check whether there are any leftover devicemapper devices from the pool.
pub fn has_leftover_devices(pool_uuid: PoolUuid) -> bool {
    let mut has_leftover = false;
    let devices = list_of_partial_pool_devices(pool_uuid);
    match get_dm().list_devices() {
        Ok(l) => {
            let listed_devices = l
                .into_iter()
                .map(|(dm_name, _, _)| dm_name)
                .collect::<Vec<_>>();
            for device in devices {
                if listed_devices.contains(&device) {
                    has_leftover |= true;
                }
            }
        }
        Err(_) => {
            for device in devices {
                if Path::new(&format!("/dev/mapper/{}", &*device)).exists() {
                    has_leftover |= true;
                }
            }
        }
    }

    has_leftover
}
