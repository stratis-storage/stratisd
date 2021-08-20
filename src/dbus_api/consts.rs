// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::dbus_api::types::InterfacesRemoved;

pub const STRATIS_BASE_PATH: &str = "/org/storage/stratis3";
pub const STRATIS_BASE_SERVICE: &str = "org.storage.stratis3";

pub const MANAGER_INTERFACE_NAME_3_0: &str = "org.storage.stratis3.Manager.r0";
pub const REPORT_INTERFACE_NAME_3_0: &str = "org.storage.stratis3.Report.r0";

pub const PROPERTY_FETCH_INTERFACE_NAME_3_0: &str = "org.storage.stratis3.FetchProperties.r0";

pub const KEY_LIST_PROP: &str = "KeyList";

pub const LOCKED_POOL_DEVS: &str = "LockedPoolsWithDevs";

pub const POOL_INTERFACE_NAME_3_0: &str = "org.storage.stratis3.pool.r0";
pub const POOL_NAME_PROP: &str = "Name";
pub const POOL_UUID_PROP: &str = "Uuid";
pub const POOL_HAS_CACHE_PROP: &str = "HasCache";
pub const POOL_ENCRYPTED_PROP: &str = "Encrypted";
pub const POOL_AVAIL_ACTIONS_PROP: &str = "AvailableActions";
pub const POOL_ENCRYPTION_KEY_DESC: &str = "KeyDescription";
pub const POOL_TOTAL_SIZE_PROP: &str = "TotalPhysicalSize";
pub const POOL_TOTAL_USED_PROP: &str = "TotalPhysicalUsed";
pub const POOL_CLEVIS_INFO: &str = "ClevisInfo";

pub const FILESYSTEM_INTERFACE_NAME_3_0: &str = "org.storage.stratis3.filesystem.r0";
pub const FILESYSTEM_NAME_PROP: &str = "Name";
pub const FILESYSTEM_UUID_PROP: &str = "Uuid";
pub const FILESYSTEM_USED_PROP: &str = "Used";
pub const FILESYSTEM_DEVNODE_PROP: &str = "Devnode";
pub const FILESYSTEM_POOL_PROP: &str = "Pool";
pub const FILESYSTEM_CREATED_PROP: &str = "Created";

pub const BLOCKDEV_INTERFACE_NAME_3_0: &str = "org.storage.stratis3.blockdev.r0";
pub const BLOCKDEV_DEVNODE_PROP: &str = "Devnode";
pub const BLOCKDEV_HARDWARE_INFO_PROP: &str = "HardwareInfo";
pub const BLOCKDEV_USER_INFO_PROP: &str = "UserInfo";
pub const BLOCKDEV_INIT_TIME_PROP: &str = "InitializationTime";
pub const BLOCKDEV_POOL_PROP: &str = "Pool";
pub const BLOCKDEV_UUID_PROP: &str = "Uuid";
pub const BLOCKDEV_TIER_PROP: &str = "Tier";
pub const BLOCKDEV_PHYSICAL_PATH_PROP: &str = "PhysicalPath";

pub const BLOCKDEV_TOTAL_SIZE_PROP: &str = "TotalPhysicalSize";
pub const BLOCKDEV_TOTAL_SIZE_ALLOCATED_PROP: &str = "TotalPhysicalSizeAllocated";
pub const BLOCKDEV_TOTAL_REAL_SIZE_PROP: &str = "TotalPhysicalRealSize";

/// Get a list of all the FetchProperties interfaces
pub fn fetch_properties_interfaces() -> Vec<String> {
    [PROPERTY_FETCH_INTERFACE_NAME_3_0]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Get a list of all the standard pool interfaces; i.e., all the revisions of
/// org.storage.stratis2.pool.
pub fn standard_pool_interfaces() -> Vec<String> {
    [POOL_INTERFACE_NAME_3_0]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Get a list of all the standard filesystem interfaces; i.e., all the
/// revisions of org.storage.stratis2.filesystem.
pub fn standard_filesystem_interfaces() -> Vec<String> {
    [FILESYSTEM_INTERFACE_NAME_3_0]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Get a list of all the standard blockdev interfaces; i.e., all the
/// revisions of org.storage.stratis2.blockdev.
pub fn standard_blockdev_interfaces() -> Vec<String> {
    [BLOCKDEV_INTERFACE_NAME_3_0]
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
