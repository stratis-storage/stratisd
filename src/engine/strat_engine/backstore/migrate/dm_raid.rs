// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
#![allow(dead_code)]

use std::{cmp::min, fs::File, path::Path, thread::sleep, time::Duration};

use devicemapper::{DevId, Device, DmOptions};

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

pub fn set_up_raid_array(
    pool_uuid: PoolUuid,
    cap_device: &Path,
    destination: &Path,
) -> StratisResult<Device> {
    let cap_dev_size = blkdev_size(&File::open(cap_device)?)?;
    let dest_dev_size = blkdev_size(&File::open(destination)?)?;
    let cap_devno = get_devno_from_path(cap_device)?;
    let dest_devno = get_devno_from_path(destination)?;

    if cap_dev_size > dest_dev_size {
        return Err(StratisError::Msg(format!("Size of destination device ({}) must be as large or larger than the cap device size ({})", dest_dev_size, cap_dev_size)));
    }

    let dm = get_dm();
    let (dm_name, dm_uuid) = format_dm_raid_migrate_ids(pool_uuid);

    let args = vec![(
        0,
        *min(cap_dev_size.sectors(), dest_dev_size.sectors()),
        "raid".to_string(),
        format!("raid1 1 0 2 - {} - {}", cap_devno, dest_devno),
    )];

    let devinfo = dm.device_create(&dm_name, Some(&dm_uuid), DmOptions::private())?;
    dm.table_load(
        &DevId::Name(&dm_name),
        args.as_slice(),
        DmOptions::default(),
    )?;
    dm.device_suspend(&DevId::Name(&dm_name), DmOptions::default())?;

    Ok(devinfo.device())
}

pub fn wait_on_sync_completion(pool_uuid: PoolUuid) -> StratisResult<()> {
    let dm = get_dm();
    let (dm_name, _) = format_dm_raid_migrate_ids(pool_uuid);

    loop {
        sleep(Duration::from_millis(100));
        let (_, table_status) = dm.table_status(&DevId::Name(&dm_name), DmOptions::private())?;
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

pub fn tear_down_raid(pool_uuid: PoolUuid) -> StratisResult<()> {
    let dm = get_dm();
    let (dm_name, _) = format_dm_raid_migrate_ids(pool_uuid);

    dm.device_remove(&DevId::Name(&dm_name), DmOptions::private())
        .map(|_| ())
        .map_err(StratisError::from)
}
