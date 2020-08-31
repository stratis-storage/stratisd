// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{env, path::PathBuf};

use uuid::Uuid;

mod lib;

fn unit_template(uuids: Vec<Uuid>, pool_uuid: Uuid) -> String {
    let devices: Vec<_> = uuids
        .into_iter()
        .map(|uuid| {
            lib::encode_path_to_device_unit(&PathBuf::from(format!("/dev/disk/by-uuid/{}", uuid)))
        })
        .collect();
    format!(
        r"[Unit]
Description=prompt for root filesystem password
{}
{}

[Service]
Type=oneshot
Environment='STRATIS_ROOTFS_UUID={}'
ExecStart=/usr/lib/systemd/stratis-key-set

[Install]
WantedBy=initrd.target
",
        format!("Requires={}", devices.join(" ")),
        format!("After={}", devices.join(" ")),
        pool_uuid,
    )
}

fn main() -> Result<(), String> {
    let (_, early_dir, _) = lib::get_generator_args()?;

    let rootfs_uuids = env::var("STRATIS_ROOTFS_UUIDS").map_err(|e| e.to_string())?;
    let pool_uuid = env::var("STRATIS_ROOTFS_POOL_UUID").map_err(|e| e.to_string())?;
    let parsed_rootfs_uuids: Vec<_> = rootfs_uuids
        .split(',')
        .filter_map(|string| Uuid::parse_str(string).ok())
        .collect();
    let parsed_pool_uuid = Uuid::parse_str(&pool_uuid).map_err(|e| e.to_string())?;
    let file_contents = unit_template(parsed_rootfs_uuids, parsed_pool_uuid);
    let mut path = PathBuf::from(early_dir);
    path.push("stratis-rootfs-prompt.service");
    lib::write_unit_file(&path, file_contents).map_err(|e| e.to_string())
}
