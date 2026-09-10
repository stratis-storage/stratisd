// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
#![allow(dead_code)]

use std::{fs::File, io::Read, path::Path, process::Command};

use futures::executor::block_on;
use nix::fcntl::{fcntl, readlink, FcntlArg, OFlag};
use tokio::io::{unix::AsyncFd, Interest};

use crate::engine::strat_engine::device::blkdev_size;

use crate::{
    engine::types::PoolUuid,
    stratis::{StratisError, StratisResult},
};

fn migrate(pool_uuid: PoolUuid, cap_device: &Path, destination: &Path) -> StratisResult<()> {
    set_up_raid_array(pool_uuid, cap_device, destination)?;
    block_on(wait_on_sync_completion(pool_uuid))?;
    tear_down_raid(pool_uuid)?;

    Ok(())
}

fn set_up_raid_array(
    pool_uuid: PoolUuid,
    cap_device: &Path,
    destination: &Path,
) -> StratisResult<()> {
    let cap_dev_size = blkdev_size(&File::open(cap_device)?)?;
    let dest_dev_size = blkdev_size(&File::open(destination)?)?;

    if cap_dev_size > dest_dev_size {
        return Err(StratisError::Msg(format!("Size of destination device ({}) must be as large or larger than the cap device size ({})", dest_dev_size, cap_dev_size)));
    }

    let mut cmd = Command::new("mdadm");
    cmd.arg("--create")
        .arg(format!("/dev/md/{pool_uuid}").as_str())
        .arg("--level=1")
        .arg("--raid-devices=2")
        .arg("--metadata=none")
        .arg(cap_device)
        .arg(destination);

    let child = cmd.spawn()?;
    let output = child.wait_with_output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(StratisError::from(std::io::Error::from_raw_os_error(
            output.status.code().unwrap_or(1),
        )))
    }
}

async fn wait_on_sync_completion(pool_uuid: PoolUuid) -> StratisResult<()> {
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
        let mut guard = async_fd.ready(Interest::PRIORITY).await?;
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

fn tear_down_raid(pool_uuid: PoolUuid) -> StratisResult<()> {
    let mut cmd = Command::new("mdadm");
    cmd.arg("--stop")
        .arg(format!("/dev/md/{pool_uuid}").as_str());

    let child = cmd.spawn()?;
    let output = child.wait_with_output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(StratisError::from(std::io::Error::from_raw_os_error(
            output.status.code().unwrap_or(1),
        )))
    }
}
