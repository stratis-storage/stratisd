// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use stratisd::engine::{FilesystemUuid, PoolUuid};

use zbus::{
    blocking::{fdo::ObjectManagerProxy, Connection},
    fdo::{Error, ManagedObjects},
    names::OwnedInterfaceName,
    zvariant::OwnedValue,
};

const REVISION_NUMBER: u8 = 9;
const BUS_NAME: &str = "org.storage.stratis3";
const TOP_OBJECT: &str = "/org/storage/stratis3";

static REVISION: LazyLock<String> = LazyLock::new(|| format!("r{REVISION_NUMBER}"));
static POOL_INTERFACE: LazyLock<String> =
    LazyLock::new(|| format!("{BUS_NAME}.pool.{}", &*REVISION));
static FILESYSTEM_INTERFACE: LazyLock<String> =
    LazyLock::new(|| format!("{BUS_NAME}.filesystem.{}", &*REVISION));

// Get managed objects for Stratis.
fn get_managed_objects() -> Result<ManagedObjects, Error> {
    let conn = Connection::system()?;
    let proxy = ObjectManagerProxy::builder(&conn)
        .destination(BUS_NAME)?
        .path(TOP_OBJECT)?
        .build()?;

    proxy.get_managed_objects()
}

// Extract the devicemapper name from the devicemapper path
fn extract_dm_name(dmpath: &Path) -> Result<String, String> {
    assert!(dmpath.is_absolute(), "parser ensures absolute path");

    match dmpath
        .components()
        .map(|c| c.as_os_str())
        .collect::<Vec<_>>()
        .as_slice()
    {
        [_, dev, mapper, name] if *dev == "dev" && *mapper == "mapper" => {
            name.to_str().map(|n| n.to_string()).ok_or_else(|| {
                format!(
                    "error converting Stratis filesystem devicemapper path name to string: {:?}",
                    name
                )
            })
        }
        _ => Err(format!(
            "error decomposing Stratis filesystem devicemapper path: {}",
            dmpath.display()
        )),
    }
}

// Parse a Stratis filesystem devicemapper name.
fn parse_dm_name(dmname: &str) -> Result<(PoolUuid, FilesystemUuid), String> {
    match dmname.split('-').collect::<Vec<_>>().as_slice() {
        [stratis, format_version, pool_uuid, thin, fs, filesystem_uuid]
            if *stratis == "stratis"
                && *format_version == "1"
                && *thin == "thin"
                && *fs == "fs" =>
        {
            Ok((
                PoolUuid::parse_str(pool_uuid).map_err(|e| e.to_string())?,
                FilesystemUuid::parse_str(filesystem_uuid).map_err(|e| e.to_string())?,
            ))
        }
        _ => Err(format!(
            "error parsing Stratis filesystem devicemapper name \"{}\"",
            dmname
        )),
    }
}

// Get the Name property by matching on Uuid property for the given interface.
fn get_name_by_uuid_and_intf(
    ifaces: &HashMap<OwnedInterfaceName, HashMap<String, OwnedValue>>,
    interface_name: &str,
    uuid_str: &str,
) -> Option<String> {
    let props = ifaces.get(interface_name)?;
    if props
        .get("Uuid")?
        .downcast_ref::<String>()
        .ok()
        .map(|uuid| uuid == uuid_str)
        .unwrap_or(false)
    {
        props.get("Name")?.downcast_ref::<String>().ok()
    } else {
        None
    }
}

// Given value of Uuid property for filesystem and pool, generate symlink.
fn get_symlink_by_uuids(
    managed_objects: &ManagedObjects,
    pool_uuid: PoolUuid,
    filesystem_uuid: FilesystemUuid,
) -> Result<PathBuf, String> {
    let (pool_uuid_str, filesystem_uuid_str) = (
        pool_uuid.simple().to_string(),
        filesystem_uuid.simple().to_string(),
    );
    match managed_objects.values().fold(
        (None::<String>, None::<String>),
        |(pool_name, filesystem_name), ifaces| match (pool_name, filesystem_name) {
            (Some(pool_name), Some(filesystem_name)) => (Some(pool_name), Some(filesystem_name)),
            (Some(pool_name), None) => (
                Some(pool_name),
                get_name_by_uuid_and_intf(ifaces, &FILESYSTEM_INTERFACE, &filesystem_uuid_str),
            ),
            (None, Some(filesystem_name)) => (
                get_name_by_uuid_and_intf(ifaces, &POOL_INTERFACE, &pool_uuid_str),
                Some(filesystem_name),
            ),
            (None, None) => (
                get_name_by_uuid_and_intf(ifaces, &POOL_INTERFACE, &pool_uuid_str),
                get_name_by_uuid_and_intf(ifaces, &FILESYSTEM_INTERFACE, &filesystem_uuid_str),
            ),
        },
    ) {
        (Some(pool_name), Some(filesystem_name)) => {
            Ok(["/", "dev", "stratis", &pool_name, &filesystem_name]
                .iter()
                .collect::<PathBuf>())
        }
        (Some(_), None) => Err(
            "Filesystem name could not be found; can not synthesize Stratis filesystem symlink"
                .to_string(),
        ),
        (None, Some(_)) => Err(
            "Pool name could not be found; can not synthesize Stratis filesystem symlink"
                .to_string(),
        ),
        _ => Err(
            "Pool name and filesystem name could not be found; can not synthesize Stratis filesystem symlink"
                .to_string(),
        )
    }
}

/// Return Stratis pool name from filesystem device mapper path.
pub fn pool_name(dm_path: &Path) -> Result<String, String> {
    let dm_name = extract_dm_name(dm_path)?;
    let (pool_uuid, _filesystem_uuid) = parse_dm_name(&dm_name)?;
    let managed_objects = get_managed_objects()
        .map_err(|e| format!("Unable to retrieve Stratis information from the D-Bus: {e}"))?;

    managed_objects
        .values()
        .find_map(|ifaces| {
            get_name_by_uuid_and_intf(ifaces, &POOL_INTERFACE, &pool_uuid.simple().to_string())
        })
        .ok_or_else(|| format!("Name for pool with UUID {pool_uuid} not found"))
}

/// Return Stratis filesystem name from filesystem device mapper path.
pub fn filesystem_name(dm_path: &Path) -> Result<String, String> {
    let dm_name = extract_dm_name(dm_path)?;
    let (_pool_uuid, filesystem_uuid) = parse_dm_name(&dm_name)?;
    let managed_objects = get_managed_objects()
        .map_err(|e| format!("Unable to retrieve Stratis information from the D-Bus: {e}"))?;

    managed_objects
        .values()
        .find_map(|ifaces| {
            get_name_by_uuid_and_intf(
                ifaces,
                &FILESYSTEM_INTERFACE,
                &filesystem_uuid.simple().to_string(),
            )
        })
        .ok_or_else(|| format!("Name for filesystem with UUID {filesystem_uuid} not found"))
}

/// Return Stratis-maintained symlink corresponding to devicemapper path
pub fn symlink(dm_path: &Path) -> Result<PathBuf, String> {
    let dm_name = extract_dm_name(dm_path)?;
    let (pool_uuid, filesystem_uuid) = parse_dm_name(&dm_name)?;
    let managed_objects = get_managed_objects()
        .map_err(|e| format!("Unable to retrieve Stratis information from the D-Bus: {e}"))?;

    get_symlink_by_uuids(&managed_objects, pool_uuid, filesystem_uuid)
}
