// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub const STRATIS_BASE_PATH: &str = "/org/storage/stratis1";
pub const STRATIS_BASE_SERVICE: &str = "org.storage.stratis1";

pub const MANAGER_INTERFACE_NAME: &str = "org.storage.stratis1.Manager";

pub const PROPERTY_FETCH_INTERFACE_NAME: &str = "org.storage.stratis1.FetchProperties";

pub const POOL_INTERFACE_NAME: &str = "org.storage.stratis1.pool";
pub const POOL_NAME_PROP: &str = "Name";
pub const POOL_STATE_PROP: &str = "State";
pub const POOL_EXTEND_STATE_PROP: &str = "ExtendState";
pub const POOL_SPACE_STATE_PROP: &str = "SpaceState";
pub const POOL_TOTAL_SIZE_PROP: &str = "TotalPhysicalSize";
pub const POOL_TOTAL_USED_PROP: &str = "TotalPhysicalUsed";

pub const FILESYSTEM_INTERFACE_NAME: &str = "org.storage.stratis1.filesystem";
pub const FILESYSTEM_NAME_PROP: &str = "Name";
pub const FILESYSTEM_USED_PROP: &str = "Used";

pub const BLOCKDEV_INTERFACE_NAME: &str = "org.storage.stratis1.blockdev";
pub const BLOCKDEV_STATE_PROP: &str = "State";
