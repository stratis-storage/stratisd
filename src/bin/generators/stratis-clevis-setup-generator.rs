// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{error::Error, path::PathBuf};

use log::info;
use uuid::Uuid;

mod lib;

fn unit_template(pool_uuid: Uuid) -> String {
    format!(
        r"[Unit]
Description=setup for Stratis root filesystem using Clevis
DefaultDependencies=no
Conflicts=shutdown.target
OnFailure=emergency.target
OnFailureJobMode=isolate
Wants=stratisd-min.service network-online.target
After=stratisd-min.service network-online.target
Before=stratis-setup.service

[Service]
Type=oneshot
Environment='STRATIS_ROOTFS_UUID={}'
ExecStart=/usr/lib/systemd/stratis-clevis-rootfs-setup
RemainAfterExit=yes
",
        pool_uuid,
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    lib::setup_logger()?;

    let (_, early_dir, _) = lib::get_generator_args()?;
    let kernel_cmdline = lib::get_kernel_cmdline()?;

    let pool_uuid_key = "stratis.rootfs.pool_uuid";
    let pool_uuid = match kernel_cmdline
        .get(pool_uuid_key)
        .and_then(|opt_vec| opt_vec.as_ref())
        .and_then(|vec| vec.iter().next())
    {
        Some(uuid) => uuid,
        None => {
            info!(
                "{} kernel command line parameter not found; disabling generator",
                pool_uuid_key
            );
            return Ok(());
        }
    };

    let parsed_pool_uuid = Uuid::parse_str(&pool_uuid)?;
    let file_contents = unit_template(parsed_pool_uuid);
    let mut path = PathBuf::from(early_dir);
    path.push("stratis-clevis-setup.service");
    lib::write_unit_file(&path, file_contents)?;
    Ok(())
}
