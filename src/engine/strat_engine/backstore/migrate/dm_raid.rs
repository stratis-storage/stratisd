// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
#![allow(dead_code)]

use std::{cmp::min, fs::File, path::Path};

use devicemapper::{DevId, DmNameBuf, DmOptions, DmUuidBuf, DM};

use crate::{
    engine::{
        strat_engine::{
            backstore::devices::get_devno_from_path, device::blkdev_size, dm::get_dm,
            names::format_dm_raid_migrate_ids,
        },
        types::PoolUuid,
    },
    stratis::{StratisError, StratisResult},
};

fn migrate(pool_uuid: PoolUuid, cap_device: &Path, destination: &Path) -> StratisResult<()> {
    let dm = get_dm();
    let (dm_name, dm_uuid) = format_dm_raid_migrate_ids(pool_uuid);
    set_up_raid_array(dm, &dm_name, &dm_uuid, cap_device, destination)?;
    wait_on_sync_completion(dm, &dm_name)?;
    tear_down_raid(dm, &dm_name)?;

    Ok(())
}

fn set_up_raid_array(
    dm: &DM,
    dm_name: &DmNameBuf,
    dm_uuid: &DmUuidBuf,
    cap_device: &Path,
    destination: &Path,
) -> StratisResult<()> {
    let cap_dev_size = blkdev_size(&File::open(cap_device)?)?;
    let dest_dev_size = blkdev_size(&File::open(destination)?)?;
    let cap_devno = get_devno_from_path(cap_device)?;
    let dest_devno = get_devno_from_path(destination)?;

    if cap_dev_size > dest_dev_size {
        return Err(StratisError::Msg(format!("Size of destination device ({}) must be as large or larger than the cap device size ({})", dest_dev_size, cap_dev_size)));
    }

    let args = vec![(
        0,
        *min(cap_dev_size.sectors(), dest_dev_size.sectors()),
        "raid".to_string(),
        format!("raid1 1 0 2 - {} - {}", cap_devno, dest_devno),
    )];

    dm.device_create(dm_name, Some(dm_uuid), DmOptions::private())?;
    dm.table_load(&DevId::Name(dm_name), args.as_slice(), DmOptions::default())?;
    dm.device_suspend(&DevId::Name(dm_name), DmOptions::default())?;

    Ok(())
}

fn wait_on_sync_completion(dm: &DM, dm_name: &DmNameBuf) -> StratisResult<()> {
    loop {
        let (_, table_status) = dm.table_status(&DevId::Name(dm_name), DmOptions::private())?;
        let mut all_statuses = Vec::new();
        for (_, _, ty, status) in table_status {
            if ty == "raid" {
                let sync_status = status.split(" ").nth(7).ok_or_else(|| {
                    StratisError::Msg(format!(
                        "dm-raid status did not have the expected format: {status}"
                    ))
                })?;
                all_statuses.push(sync_status.to_string());
            }
        }
        if all_statuses.iter().all(|s| s == "idle") {
            break;
        }
    }

    Ok(())
}

fn tear_down_raid(dm: &DM, dm_name: &DmNameBuf) -> StratisResult<()> {
    dm.device_remove(&DevId::Name(dm_name), DmOptions::private())
        .map(|_| ())
        .map_err(StratisError::from)
}
