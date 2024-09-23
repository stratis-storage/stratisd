// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::dbus_api::types::InterfacesRemoved;

pub const STRATIS_BASE_PATH: &str = "/org/storage/stratis3";
pub const STRATIS_BASE_SERVICE: &str = "org.storage.stratis3";

pub const MANAGER_INTERFACE_NAME_3_0: &str = "org.storage.stratis3.Manager.r0";
pub const MANAGER_INTERFACE_NAME_3_1: &str = "org.storage.stratis3.Manager.r1";
pub const MANAGER_INTERFACE_NAME_3_2: &str = "org.storage.stratis3.Manager.r2";
pub const MANAGER_INTERFACE_NAME_3_3: &str = "org.storage.stratis3.Manager.r3";
pub const MANAGER_INTERFACE_NAME_3_4: &str = "org.storage.stratis3.Manager.r4";
pub const MANAGER_INTERFACE_NAME_3_5: &str = "org.storage.stratis3.Manager.r5";
pub const MANAGER_INTERFACE_NAME_3_6: &str = "org.storage.stratis3.Manager.r6";
pub const MANAGER_INTERFACE_NAME_3_7: &str = "org.storage.stratis3.Manager.r7";
pub const REPORT_INTERFACE_NAME_3_0: &str = "org.storage.stratis3.Report.r0";
pub const REPORT_INTERFACE_NAME_3_1: &str = "org.storage.stratis3.Report.r1";
pub const REPORT_INTERFACE_NAME_3_2: &str = "org.storage.stratis3.Report.r2";
pub const REPORT_INTERFACE_NAME_3_3: &str = "org.storage.stratis3.Report.r3";
pub const REPORT_INTERFACE_NAME_3_4: &str = "org.storage.stratis3.Report.r4";
pub const REPORT_INTERFACE_NAME_3_5: &str = "org.storage.stratis3.Report.r5";
pub const REPORT_INTERFACE_NAME_3_6: &str = "org.storage.stratis3.Report.r6";
pub const REPORT_INTERFACE_NAME_3_7: &str = "org.storage.stratis3.Report.r7";

pub const LOCKED_POOLS_PROP: &str = "LockedPools";
pub const STOPPED_POOLS_PROP: &str = "StoppedPools";

pub const POOL_INTERFACE_NAME_3_0: &str = "org.storage.stratis3.pool.r0";
pub const POOL_INTERFACE_NAME_3_1: &str = "org.storage.stratis3.pool.r1";
pub const POOL_INTERFACE_NAME_3_2: &str = "org.storage.stratis3.pool.r2";
pub const POOL_INTERFACE_NAME_3_3: &str = "org.storage.stratis3.pool.r3";
pub const POOL_INTERFACE_NAME_3_4: &str = "org.storage.stratis3.pool.r4";
pub const POOL_INTERFACE_NAME_3_5: &str = "org.storage.stratis3.pool.r5";
pub const POOL_INTERFACE_NAME_3_6: &str = "org.storage.stratis3.pool.r6";
pub const POOL_INTERFACE_NAME_3_7: &str = "org.storage.stratis3.pool.r7";
pub const POOL_NAME_PROP: &str = "Name";
pub const POOL_UUID_PROP: &str = "Uuid";
pub const POOL_HAS_CACHE_PROP: &str = "HasCache";
pub const POOL_ENCRYPTED_PROP: &str = "Encrypted";
pub const POOL_AVAIL_ACTIONS_PROP: &str = "AvailableActions";
pub const POOL_KEY_DESC_PROP: &str = "KeyDescription";
pub const POOL_TOTAL_SIZE_PROP: &str = "TotalPhysicalSize";
pub const POOL_TOTAL_USED_PROP: &str = "TotalPhysicalUsed";
pub const POOL_CLEVIS_INFO_PROP: &str = "ClevisInfo";
pub const POOL_ALLOC_SIZE_PROP: &str = "AllocatedSize";
pub const POOL_FS_LIMIT_PROP: &str = "FsLimit";
pub const POOL_OVERPROV_PROP: &str = "Overprovisioning";
pub const POOL_NO_ALLOCABLE_SPACE_PROP: &str = "NoAllocSpace";

pub const FILESYSTEM_INTERFACE_NAME_3_0: &str = "org.storage.stratis3.filesystem.r0";
pub const FILESYSTEM_INTERFACE_NAME_3_1: &str = "org.storage.stratis3.filesystem.r1";
pub const FILESYSTEM_INTERFACE_NAME_3_2: &str = "org.storage.stratis3.filesystem.r2";
pub const FILESYSTEM_INTERFACE_NAME_3_3: &str = "org.storage.stratis3.filesystem.r3";
pub const FILESYSTEM_INTERFACE_NAME_3_4: &str = "org.storage.stratis3.filesystem.r4";
pub const FILESYSTEM_INTERFACE_NAME_3_5: &str = "org.storage.stratis3.filesystem.r5";
pub const FILESYSTEM_INTERFACE_NAME_3_6: &str = "org.storage.stratis3.filesystem.r6";
pub const FILESYSTEM_INTERFACE_NAME_3_7: &str = "org.storage.stratis3.filesystem.r7";
pub const FILESYSTEM_NAME_PROP: &str = "Name";
pub const FILESYSTEM_UUID_PROP: &str = "Uuid";
pub const FILESYSTEM_USED_PROP: &str = "Used";
pub const FILESYSTEM_DEVNODE_PROP: &str = "Devnode";
pub const FILESYSTEM_POOL_PROP: &str = "Pool";
pub const FILESYSTEM_CREATED_PROP: &str = "Created";
pub const FILESYSTEM_SIZE_PROP: &str = "Size";
pub const FILESYSTEM_SIZE_LIMIT_PROP: &str = "SizeLimit";

pub const BLOCKDEV_INTERFACE_NAME_3_0: &str = "org.storage.stratis3.blockdev.r0";
pub const BLOCKDEV_INTERFACE_NAME_3_1: &str = "org.storage.stratis3.blockdev.r1";
pub const BLOCKDEV_INTERFACE_NAME_3_2: &str = "org.storage.stratis3.blockdev.r2";
pub const BLOCKDEV_INTERFACE_NAME_3_3: &str = "org.storage.stratis3.blockdev.r3";
pub const BLOCKDEV_INTERFACE_NAME_3_4: &str = "org.storage.stratis3.blockdev.r4";
pub const BLOCKDEV_INTERFACE_NAME_3_5: &str = "org.storage.stratis3.blockdev.r5";
pub const BLOCKDEV_INTERFACE_NAME_3_6: &str = "org.storage.stratis3.blockdev.r6";
pub const BLOCKDEV_INTERFACE_NAME_3_7: &str = "org.storage.stratis3.blockdev.r7";
pub const BLOCKDEV_DEVNODE_PROP: &str = "Devnode";
pub const BLOCKDEV_HARDWARE_INFO_PROP: &str = "HardwareInfo";
pub const BLOCKDEV_USER_INFO_PROP: &str = "UserInfo";
pub const BLOCKDEV_INIT_TIME_PROP: &str = "InitializationTime";
pub const BLOCKDEV_POOL_PROP: &str = "Pool";
pub const BLOCKDEV_UUID_PROP: &str = "Uuid";
pub const BLOCKDEV_TIER_PROP: &str = "Tier";
pub const BLOCKDEV_PHYSICAL_PATH_PROP: &str = "PhysicalPath";
pub const BLOCKDEV_NEW_SIZE_PROP: &str = "NewPhysicalSize";
pub const BLOCKDEV_TOTAL_SIZE_PROP: &str = "TotalPhysicalSize";

/// Get a list of all the standard pool interfaces
pub fn standard_pool_interfaces() -> Vec<String> {
    [
        POOL_INTERFACE_NAME_3_0,
        POOL_INTERFACE_NAME_3_1,
        POOL_INTERFACE_NAME_3_2,
        POOL_INTERFACE_NAME_3_3,
        POOL_INTERFACE_NAME_3_4,
        POOL_INTERFACE_NAME_3_5,
        POOL_INTERFACE_NAME_3_6,
        POOL_INTERFACE_NAME_3_7,
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

/// Get a list of all the standard filesystem interfaces
pub fn standard_filesystem_interfaces() -> Vec<String> {
    [
        FILESYSTEM_INTERFACE_NAME_3_0,
        FILESYSTEM_INTERFACE_NAME_3_1,
        FILESYSTEM_INTERFACE_NAME_3_2,
        FILESYSTEM_INTERFACE_NAME_3_3,
        FILESYSTEM_INTERFACE_NAME_3_4,
        FILESYSTEM_INTERFACE_NAME_3_5,
        FILESYSTEM_INTERFACE_NAME_3_6,
        FILESYSTEM_INTERFACE_NAME_3_7,
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

/// Get a list of all the standard blockdev interfaces
pub fn standard_blockdev_interfaces() -> Vec<String> {
    [
        BLOCKDEV_INTERFACE_NAME_3_0,
        BLOCKDEV_INTERFACE_NAME_3_1,
        BLOCKDEV_INTERFACE_NAME_3_2,
        BLOCKDEV_INTERFACE_NAME_3_3,
        BLOCKDEV_INTERFACE_NAME_3_4,
        BLOCKDEV_INTERFACE_NAME_3_5,
        BLOCKDEV_INTERFACE_NAME_3_6,
        BLOCKDEV_INTERFACE_NAME_3_7,
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect()
}

/// Get a list of all interfaces supported by a pool object.
pub fn pool_interface_list() -> InterfacesRemoved {
    standard_pool_interfaces()
}

/// Get a list of all interfaces supported by a filesystem object.
pub fn filesystem_interface_list() -> InterfacesRemoved {
    standard_filesystem_interfaces()
}

/// Get a list of all interfaces supported by a blockdev object.
pub fn blockdev_interface_list() -> InterfacesRemoved {
    standard_blockdev_interfaces()
}
