// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    error::Error,
    fs::create_dir_all,
    io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

use log::{error, warn};
use uuid::Uuid;

use super::lib;

const WANTED_BY_INITRD_PATH: &str = "/run/systemd/system/initrd.target.wants";

fn make_wanted_by_initrd(unit_path: &Path) -> Result<(), io::Error> {
    let initrd_target_wants_path = &Path::new(WANTED_BY_INITRD_PATH);
    if !initrd_target_wants_path.exists() {
        create_dir_all(initrd_target_wants_path)?;
    }
    symlink(
        unit_path,
        [
            initrd_target_wants_path,
            &Path::new(unit_path.file_name().expect("Is unit file")),
        ]
        .iter()
        .collect::<PathBuf>(),
    )?;
    Ok(())
}

fn unit_template(pool_uuid: Uuid) -> String {
    format!(
        r"[Unit]
Description=setup for Stratis root filesystem
DefaultDependencies=no
Conflicts=shutdown.target
OnFailure=emergency.target
OnFailureJobMode=isolate
Wants=stratisd-min.service plymouth-start.service stratis-clevis-setup.service
After=paths.target plymouth-start.service stratisd-min.service
Before=initrd.target

[Service]
Type=oneshot
Environment='STRATIS_ROOTFS_UUID={}'
ExecStart=/usr/lib/systemd/stratis-rootfs-setup
RemainAfterExit=yes
",
        pool_uuid,
    )
}

fn generator_with_err(early_dir: String) -> Result<(), Box<dyn Error>> {
    let kernel_cmdline = lib::get_kernel_cmdline()?;

    let pool_uuid_key = "stratis.rootfs.pool_uuid";
    let pool_uuid = match kernel_cmdline
        .get(pool_uuid_key)
        .and_then(|opt_vec| opt_vec.as_ref())
        .and_then(|vec| vec.iter().next())
    {
        Some(uuid) => uuid,
        None => {
            warn!(
                "{} kernel command line parameter not found; disabling generator",
                pool_uuid_key
            );
            return Ok(());
        }
    };

    let parsed_pool_uuid = Uuid::parse_str(&pool_uuid)?;
    let file_contents = unit_template(parsed_pool_uuid);
    let mut path = PathBuf::from(early_dir);
    path.push("stratis-setup.service");
    lib::write_unit_file(&path, file_contents)?;
    make_wanted_by_initrd(&path)?;

    Ok(())
}

pub fn generator(early_dir: String) -> Result<(), Box<dyn Error>> {
    lib::setup_logger()?;

    let res = generator_with_err(early_dir);
    if let Err(ref e) = res {
        error!("systemd generator failed with error: {}", e);
    }
    res
}
