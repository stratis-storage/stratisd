// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use uuid::Uuid;

mod lib;

fn unit_template(uuids: Vec<Uuid>) -> String {
    let devices: Vec<_> = uuids
        .into_iter()
        .map(|uuid| {
            lib::encode_path_to_device_unit(&PathBuf::from(format!("/dev/disk/by-uuid/{}", uuid)))
        })
        .collect();
    format!(
        r"[Unit]
Description=prompt for root filesystem password
Requires=stratis-rootfs-prompt.service
After=stratis-rootfs-prompt.service
{}
{}

[Service]
Type=oneshot
ExecStart=/usr/bin/stratis-min pool setup

[Install]
WantedBy=initrd.target
",
        format!("Requires={}", devices.join(" ")),
        format!("After={}", devices.join(" ")),
    )
}

fn main() -> Result<(), String> {
    let (_, early_dir, _) = lib::get_generator_args()?;
    let kernel_cmdline = lib::get_kernel_cmdline().map_err(|e| e.to_string())?;

    let rootfs_uuids = kernel_cmdline
        .get("stratis.rootfs.uuids")
        .and_then(|opt_s| opt_s.as_ref().map(|s| s.to_string()))
        .ok_or_else(|| "Missing kernel command line parameter stratis.rootfs.uuids".to_string())?;
    let parsed_rootfs_uuids: Vec<_> = rootfs_uuids
        .split(',')
        .filter_map(|string| Uuid::parse_str(string).ok())
        .collect();
    let file_contents = unit_template(parsed_rootfs_uuids);
    let mut path = PathBuf::from(early_dir);
    path.push("stratis-setup.service");
    lib::write_unit_file(&path, file_contents).map_err(|e| e.to_string())
}
