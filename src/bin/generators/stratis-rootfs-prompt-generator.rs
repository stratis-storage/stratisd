// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use uuid::Uuid;

mod lib;

fn unit_template(uuids: Vec<PathBuf>, pool_uuid: Uuid) -> String {
    let devices: Vec<_> = uuids
        .into_iter()
        .map(|uuid_path| lib::encode_path_to_device_unit(&uuid_path))
        .collect();
    format!(
        r"[Unit]
Description=prompt for root filesystem password
Requires=systemd-ask-password-console.service
After=systemd-ask-password-console.service
{}
{}

[Service]
Type=oneshot
Environment='STRATIS_ROOTFS_UUID={}'
ExecStart=/usr/lib/systemd/stratis-key-set
",
        format!("Requires={}", devices.join(" ")),
        format!("After={}", devices.join(" ")),
        pool_uuid,
    )
}

fn main() -> Result<(), String> {
    let (_, early_dir, _) = lib::get_generator_args()?;
    let kernel_cmdline = lib::get_kernel_cmdline().map_err(|e| e.to_string())?;

    let rootfs_uuid_paths = kernel_cmdline
        .get("stratis.rootfs.uuid_paths")
        .and_then(|opt_s| opt_s.as_ref().map(|s| s.to_string()))
        .ok_or_else(|| "Missing kernel command line parameter stratis.rootfs.uuids".to_string())?;
    let pool_uuid = kernel_cmdline
        .get("stratis.rootfs.pool_uuid")
        .and_then(|opt_s| opt_s.as_ref().map(|s| s.to_string()))
        .ok_or_else(|| {
            "Missing kernel command line parameter stratis.rootfs.pool_uuid".to_string()
        })?;
    let parsed_rootfs_uuid_paths: Vec<_> =
        rootfs_uuid_paths.split(',').map(PathBuf::from).collect();
    let parsed_pool_uuid = Uuid::parse_str(&pool_uuid).map_err(|e| e.to_string())?;
    let file_contents = unit_template(parsed_rootfs_uuid_paths, parsed_pool_uuid);
    let mut path = PathBuf::from(early_dir);
    path.push("stratis-rootfs-prompt.service");
    lib::write_unit_file(&path, file_contents).map_err(|e| e.to_string())?;
    lib::make_wanted_by_initrd(&path).map_err(|e| e.to_string())
}
