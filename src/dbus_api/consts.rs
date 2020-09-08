// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::dbus_api::types::InterfacesRemoved;

pub const STRATIS_BASE_PATH: &str = "/org/storage/stratis2";
pub const STRATIS_BASE_SERVICE: &str = "org.storage.stratis2";

pub const MANAGER_INTERFACE_NAME: &str = "org.storage.stratis2.Manager";
pub const MANAGER_INTERFACE_NAME_2_1: &str = "org.storage.stratis2.Manager.r1";
pub const MANAGER_INTERFACE_NAME_2_2: &str = "org.storage.stratis2.Manager.r2";
pub const REPORT_INTERFACE_NAME_2_1: &str = "org.storage.stratis2.Report.r1";

pub const PROPERTY_FETCH_INTERFACE_NAME: &str = "org.storage.stratis2.FetchProperties";
pub const PROPERTY_FETCH_INTERFACE_NAME_2_1: &str = "org.storage.stratis2.FetchProperties.r1";
pub const PROPERTY_FETCH_INTERFACE_NAME_2_2: &str = "org.storage.stratis2.FetchProperties.r2";

pub const KEY_LIST_PROP: &str = "KeyList";

pub const LOCKED_POOL_UUIDS: &str = "LockedPoolUuids";
pub const LOCKED_POOLS: &str = "LockedPools";

pub const POOL_INTERFACE_NAME: &str = "org.storage.stratis2.pool";
pub const POOL_INTERFACE_NAME_2_1: &str = "org.storage.stratis2.pool.r1";
pub const POOL_NAME_PROP: &str = "Name";
pub const POOL_UUID_PROP: &str = "Uuid";
pub const POOL_HAS_CACHE_PROP: &str = "HasCache";
pub const POOL_ENCRYPTED_PROP: &str = "Encrypted";
pub const POOL_ENCRYPTION_KEY_DESC: &str = "KeyDescription";
pub const POOL_TOTAL_SIZE_PROP: &str = "TotalPhysicalSize";
pub const POOL_TOTAL_USED_PROP: &str = "TotalPhysicalUsed";

pub const FILESYSTEM_INTERFACE_NAME: &str = "org.storage.stratis2.filesystem";
pub const FILESYSTEM_NAME_PROP: &str = "Name";
pub const FILESYSTEM_UUID_PROP: &str = "Uuid";
pub const FILESYSTEM_USED_PROP: &str = "Used";
pub const FILESYSTEM_DEVNODE_PROP: &str = "Devnode";
pub const FILESYSTEM_POOL_PROP: &str = "Pool";
pub const FILESYSTEM_CREATED_PROP: &str = "Created";

pub const BLOCKDEV_INTERFACE_NAME: &str = "org.storage.stratis2.blockdev";
pub const BLOCKDEV_INTERFACE_NAME_2_2: &str = "org.storage.stratis2.blockdev.r2";
pub const BLOCKDEV_DEVNODE_PROP: &str = "Devnode";
pub const BLOCKDEV_HARDWARE_INFO_PROP: &str = "HardwareInfo";
pub const BLOCKDEV_USER_INFO_PROP: &str = "UserInfo";
pub const BLOCKDEV_INIT_TIME_PROP: &str = "InitializationTime";
pub const BLOCKDEV_POOL_PROP: &str = "Pool";
pub const BLOCKDEV_UUID_PROP: &str = "Uuid";
pub const BLOCKDEV_TIER_PROP: &str = "Tier";
pub const BLOCKDEV_PHYSICAL_PATH_PROP: &str = "PhysicalPath";

pub const BLOCKDEV_TOTAL_SIZE_PROP: &str = "TotalPhysicalSize";

/// Get a list of all the FetchProperties interfaces
pub fn fetch_properties_interfaces() -> Vec<String> {
    [
        PROPERTY_FETCH_INTERFACE_NAME,
        PROPERTY_FETCH_INTERFACE_NAME_2_1,
        PROPERTY_FETCH_INTERFACE_NAME_2_2,
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

// Get a list of all the standard pool interfaces; i.e., all the revisions of
// org.storage.stratis2.pool.
fn standard_pool_interfaces() -> Vec<String> {
    [POOL_INTERFACE_NAME, POOL_INTERFACE_NAME_2_1]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

// Get a list of all the standard filesystem interfaces; i.e., all the
// revisions of org.storage.stratis2.filesystem.
fn standard_filesystem_interfaces() -> Vec<String> {
    [FILESYSTEM_INTERFACE_NAME]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

// Get a list of all the standard blockdev interfaces; i.e., all the
// revisions of org.storage.stratis2.blockdev.
fn standard_blockdev_interfaces() -> Vec<String> {
    [BLOCKDEV_INTERFACE_NAME]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Get a list of all interfaces supported by a pool object.
pub fn pool_interface_list() -> InterfacesRemoved {
    let mut interfaces = standard_pool_interfaces();
    interfaces.extend(fetch_properties_interfaces());
    interfaces
}

/// Get a list of all interfaces supported by a filesystem object.
pub fn filesystem_interface_list() -> InterfacesRemoved {
    let mut interfaces = standard_filesystem_interfaces();
    interfaces.extend(fetch_properties_interfaces());
    interfaces
}

/// Get a list of all interfaces supported by a blockdev object.
pub fn blockdev_interface_list() -> InterfacesRemoved {
    let mut interfaces = standard_blockdev_interfaces();
    interfaces.extend(fetch_properties_interfaces());
    interfaces
}
