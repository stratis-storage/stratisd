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

use log::info;
use uuid::Uuid;

mod lib;

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

fn encode_path_to_device_unit(path: &Path) -> String {
    let mut encoded_path =
        path.display()
            .to_string()
            .chars()
            .skip(1)
            .fold(String::new(), |mut acc, c| {
                if c.is_alphanumeric() || c == '_' {
                    acc.push(c);
                } else if c == '/' {
                    acc.push('-');
                } else {
                    let buffer = &mut [0; 4];
                    let encoded_buffer = c.encode_utf8(buffer).as_bytes();
                    for byte in encoded_buffer.iter() {
                        acc += format!(r"\x{:x}", byte).as_str();
                    }
                }
                acc
            });
    encoded_path += ".device";
    encoded_path
}

fn unit_template(uuids: Vec<PathBuf>, pool_uuid: Uuid) -> String {
    let devices: Vec<_> = uuids
        .into_iter()
        .map(|uuid_path| encode_path_to_device_unit(&uuid_path))
        .collect();
    format!(
        r"[Unit]
Description=setup for Stratis root filesystem
DefaultDependencies=no
Conflicts=shutdown.target
OnFailure=emergency.target
OnFailureJobMode=isolate
Wants=stratisd-min.service plymouth-start.service stratis-clevis-setup.service
After=paths.target plymouth-start.service stratisd-min.service {}
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
    lib::setup_logger()?;

    let (_, early_dir, _) = lib::get_generator_args()?;
    let kernel_cmdline = lib::get_kernel_cmdline()?;

    let rootfs_uuid_paths_key = "stratis.rootfs.uuid_paths";
    let rootfs_uuid_paths = match kernel_cmdline
        .get(rootfs_uuid_paths_key)
        .and_then(|opt_s| opt_s.as_ref())
    {
        Some(paths) => paths,
        None => {
            info!(
                "{} kernel command line parameter not found; disabling generator",
                rootfs_uuid_paths_key
            );
            return Ok(());
        }
    };

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

    let rootfs_uuid_paths_parsed = rootfs_uuid_paths.iter().map(PathBuf::from).collect();
    let parsed_pool_uuid = Uuid::parse_str(&pool_uuid)?;
    let file_contents = unit_template(rootfs_uuid_paths_parsed, parsed_pool_uuid);
    let mut path = PathBuf::from(early_dir);
    path.push("stratis-setup.service");
    lib::write_unit_file(&path, file_contents)?;
    make_wanted_by_initrd(&path)?;
    Ok(())
}
