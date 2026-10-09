// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
#![allow(dead_code)]

use std::{fs::File, io::Read, path::Path};

use futures::executor::block_on;
use nix::fcntl::{fcntl, readlink, FcntlArg, OFlag};
use tokio::io::{unix::AsyncFd, Interest};

use devicemapper::Device;

use crate::{
    engine::{
        strat_engine::{backstore::devices::get_devno_from_path, cmd, device::blkdev_size},
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

    if cap_dev_size > dest_dev_size {
        return Err(StratisError::Msg(format!("Size of destination device ({}) must be as large or larger than the cap device size ({})", dest_dev_size, cap_dev_size)));
    }

    let path = cmd::set_up_raid_1(pool_uuid, cap_device, destination)?;
    get_devno_from_path(&path)
}

pub fn wait_on_sync_completion(pool_uuid: PoolUuid) -> StratisResult<()> {
    let md_dev = readlink(format!("/dev/md/{pool_uuid}").as_str())?;
    let md_dev_file_name = Path::new(&md_dev)
        .file_name()
        .ok_or_else(|| StratisError::Msg(format!("Failed to read symlink /dev/md/{pool_uuid}")))?;
    let md_status_file = File::open(format!("/sys/block/{}", md_dev_file_name.display()))?;
    let flags = OFlag::from_bits(fcntl(&md_status_file, FcntlArg::F_GETFL)?).ok_or_else(|| {
        StratisError::Msg(format!(
            "Failed to get sysfs file /sys/block/{} file descriptor flags",
            md_dev_file_name.display()
        ))
    })?;
    fcntl(
        &md_status_file,
        FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK),
    )?;
    let async_fd = AsyncFd::new(md_status_file)?;
    loop {
        let mut guard = block_on(async_fd.ready(Interest::PRIORITY))?;
        match guard.try_io(|fd| {
            let mut string = String::new();
            fd.get_ref().read_to_string(&mut string)?;
            Ok(string)
        }) {
            Ok(Ok(string)) => {
                if string == "idle" {
                    return Ok(());
                }
            }
            Ok(Err(e)) => {
                return Err(StratisError::from(e));
            }
            _ => (),
        }
    }
}

pub fn tear_down_raid(pool_uuid: PoolUuid) -> StratisResult<()> {
    cmd::tear_down_raid_1(pool_uuid)
}
