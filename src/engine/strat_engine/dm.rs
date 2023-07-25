// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Get ability to instantiate a devicemapper context.

use std::{path::Path, sync::Once};

use devicemapper::{DevId, DmNameBuf, DmOptions, DmResult, DM};

use crate::{
    engine::{
        strat_engine::names::{
            format_backstore_ids, format_crypt_name, format_flex_ids, format_thin_ids,
            format_thinpool_ids, CacheRole, FlexRole, ThinPoolRole, ThinRole,
        },
        types::{DevUuid, FilesystemUuid, PoolUuid},
    },
    stratis::{StratisError, StratisResult},
};

static INIT: Once = Once::new();
static mut DM_CONTEXT: Option<DmResult<DM>> = None;

/// Path to logical devices for encrypted devices
pub const DEVICEMAPPER_PATH: &str = "/dev/mapper";

pub fn get_dm_init() -> StratisResult<&'static DM> {
    unsafe {
        INIT.call_once(|| DM_CONTEXT = Some(DM::new()));
        match &DM_CONTEXT {
            Some(Ok(ref context)) => Ok(context),
            Some(Err(e)) => Err(StratisError::Chained(
                "Failed to initialize DM context".to_string(),
                Box::new(e.clone().into()),
            )),
            _ => panic!("DM_CONTEXT.is_some()"),
        }
    }
}

pub fn get_dm() -> &'static DM {
    get_dm_init().expect(
        "the engine has already called get_dm_init() and exited if get_dm_init() returned an error",
    )
}

pub fn remove_optional_devices(devs: Vec<DmNameBuf>) -> StratisResult<bool> {
    let mut did_something = false;
    let devices = get_dm()
        .list_devices()?
        .into_iter()
        .map(|(name, _, _)| name)
        .collect::<Vec<_>>();
    for device in devs {
        if devices.contains(&device) {
            did_something = true;
            get_dm().device_remove(&DevId::Name(&device), DmOptions::default())?;
        }
    }
    Ok(did_something)
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

pub fn list_of_backstore_devices(pool_uuid: PoolUuid) -> Vec<DmNameBuf> {
    let mut devs = Vec::new();

    let (cache, _) = format_backstore_ids(pool_uuid, CacheRole::Cache);
    devs.push(cache);
    let (cache_sub, _) = format_backstore_ids(pool_uuid, CacheRole::CacheSub);
    devs.push(cache_sub);
    let (cache_meta, _) = format_backstore_ids(pool_uuid, CacheRole::MetaSub);
    devs.push(cache_meta);
    let (origin, _) = format_backstore_ids(pool_uuid, CacheRole::OriginSub);
    devs.push(origin);

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
fn list_of_partial_pool_devices(pool_uuid: PoolUuid, dev_uuids: &[DevUuid]) -> Vec<DmNameBuf> {
    let mut devs = Vec::new();

    devs.extend(list_of_thin_pool_devices(pool_uuid));

    devs.push(mdv_device(pool_uuid));

    devs.extend(list_of_backstore_devices(pool_uuid));

    devs.extend(list_of_crypt_devices(dev_uuids));

    devs
}

/// Check whether there are any leftover devicemapper devices from the pool.
pub fn has_leftover_devices(pool_uuid: PoolUuid, dev_uuids: &[DevUuid]) -> bool {
    let mut has_leftover = false;
    let devices = list_of_partial_pool_devices(pool_uuid, dev_uuids);
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
