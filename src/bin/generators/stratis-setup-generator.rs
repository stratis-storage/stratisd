// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{error::Error, path::PathBuf};

use uuid::Uuid;

mod lib;

fn unit_template(uuids: Vec<PathBuf>, pool_uuid: Uuid) -> String {
    let devices: Vec<_> = uuids
        .into_iter()
        .map(|uuid_path| lib::encode_path_to_device_unit(&uuid_path))
        .collect();
    format!(
        r"[Unit]
Description=setup for Stratis root filesystem
DefaultDependencies=no
Conflicts=shutdown.target
OnFailure=dracut-emergency.service
Wants=stratisd-min.service plymouth-start.service network-online.target
After=paths.target plymouth-start.service stratisd-min.service network-online.target {}
Before=initrd.target
{}

[Service]
Type=oneshot
Environment='STRATIS_ROOTFS_UUID={}'
ExecStart=/usr/lib/systemd/stratis-rootfs-setup
RemainAfterExit=yes
",
        devices.join(" "),
        format!("Requires={}", devices.join(" ")),
        pool_uuid,
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let (_, early_dir, _) = lib::get_generator_args()?;
    let kernel_cmdline = lib::get_kernel_cmdline()?;

    let rootfs_uuid_paths_key = "stratis.rootfs.uuid_paths";
    let rootfs_uuid_paths = kernel_cmdline
        .get(rootfs_uuid_paths_key)
        .and_then(|opt_s| opt_s.as_ref())
        .ok_or_else(|| {
            format!(
                "Missing kernel command line parameter {}",
                rootfs_uuid_paths_key
            )
        })?;
    let rootfs_uuid_paths_parsed = rootfs_uuid_paths.iter().map(PathBuf::from).collect();
    let pool_uuid_key = "stratis.rootfs.pool_uuid";
    let pool_uuid = kernel_cmdline
        .get(pool_uuid_key)
        .and_then(|opt_vec| opt_vec.as_ref())
        .and_then(|vec| vec.iter().next())
        .ok_or_else(|| format!("Missing kernel command line parameter {}", pool_uuid_key))?;
    let parsed_pool_uuid = Uuid::parse_str(&pool_uuid)?;
    let file_contents = unit_template(rootfs_uuid_paths_parsed, parsed_pool_uuid);
    let mut path = PathBuf::from(early_dir);
    path.push("stratis-setup.service");
    lib::write_unit_file(&path, file_contents)?;
    lib::make_wanted_by_initrd(&path)?;
    Ok(())
}
