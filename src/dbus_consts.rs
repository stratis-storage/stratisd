// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub const DBUS_TIMEOUT: i32 = 20000; // millieconds

pub const STRATIS_VERSION: &'static str = "1";
pub const MANAGER_NAME: &'static str = "/Manager";
pub const STRATIS_BASE_PATH: &'static str = "/org/storage/stratis1";
pub const STRATIS_BASE_SERVICE: &'static str = "org.storage.stratis1";
pub const STRATIS_BASE_MANAGER: &'static str = "/org/storage/stratis1/Manager";
pub const STRATIS_MANAGER_INTERFACE: &'static str = "org.storage.stratis1.Manager";
pub const STRATIS_POOL_BASE_INTERFACE: &'static str = "org.storage.stratis1.pool";
pub const STRATIS_FILESYSTEM_BASE_INTERFACE: &'static str = "org.storage.stratis1.filesystem";
pub const STRATIS_DEV_BASE_INTERFACE: &'static str = "org.storage.stratis1.dev";
pub const STRATIS_CACHE_BASE_INTERFACE: &'static str = "org.storage.stratis1.cache";
pub const STRATIS_POOL_BASE_PATH: &'static str = "/org/storage/stratis/pool";

pub const DEFAULT_OBJECT_PATH: &'static str = "/";

// Manager Methods
pub const LIST_POOLS: &'static str = "ListPools";
pub const CREATE_POOL: &'static str = "CreatePool";
pub const DESTROY_POOL: &'static str = "DestroyPool";
pub const GET_POOL_OBJECT_PATH: &'static str = "GetPoolObjectPath";
pub const GET_FILESYSTEM_OBJECT_PATH: &'static str = "GetFilesystemObjectPath";
pub const GET_DEV_OBJECT_PATH: &'static str = "GetDevObjectPath";
pub const GET_CACHE_OBJECT_PATH: &'static str = "GetCacheObjectPath";
pub const GET_ERROR_CODES: &'static str = "GetErrorCodes";
pub const GET_RAID_LEVELS: &'static str = "GetRaidLevels";
pub const GET_DEV_TYPES: &'static str = "GetDevTypes";

// Pool Methods
pub const CREATE_FILESYSTEMS: &'static str = "CreateFilesystems";
pub const DESTROY_FILESYSTEMS: &'static str = "DestroyFilesystems";
pub const LIST_FILESYSTEMS: &'static str = "ListFilesystems";
pub const LIST_DEVS: &'static str = "ListDevs";
pub const LIST_CACHE_DEVS: &'static str = "ListCacheDevs";
pub const ADD_CACHE_DEVS: &'static str = "AddCacheDevs";
pub const REMOVE_CACHE_DEVS: &'static str = "RemoveCacheDevs";
pub const ADD_DEVS: &'static str = "AddDevs";
pub const REMOVE_DEVS: &'static str = "RemoveDevs";

pub trait HasCodes {
    /// Indicates that this enum can be converted to an int or described
    /// with a string.
    fn get_error_int(&self) -> u16;
    fn get_error_string(&self) -> &str;
}

custom_derive! {
    #[derive(Copy, Clone, EnumDisplay,
             IterVariants(StratisDBusErrorVariants),
             IterVariantNames(StratisDBusErrorVariantNames))]
    #[allow(non_camel_case_types)]
    pub enum StratisErrorEnum {
        STRATIS_OK,
        STRATIS_ERROR,

        STRATIS_ALREADY_EXISTS,
        STRATIS_BAD_PARAM,
        STRATIS_CACHE_NOTFOUND,
        STRATIS_DEV_NOTFOUND,
        STRATIS_FILESYSTEM_NOTFOUND,
        STRATIS_LIST_FAILURE,
        STRATIS_NO_POOLS,
        STRATIS_NOTFOUND,
        STRATIS_NULL,
        STRATIS_NULL_NAME,
        STRATIS_POOL_NOTFOUND,
    }
}

impl HasCodes for StratisErrorEnum {
    fn get_error_int(&self) -> u16 {
        *self as u16
    }

    fn get_error_string(&self) -> &str {
        match *self {
            StratisErrorEnum::STRATIS_OK => "Ok",
            StratisErrorEnum::STRATIS_ERROR => "A general error happened",
            StratisErrorEnum::STRATIS_NULL => "Null parameter was supplied",
            StratisErrorEnum::STRATIS_NOTFOUND => "Not found",
            StratisErrorEnum::STRATIS_POOL_NOTFOUND => "Pool not found",
            StratisErrorEnum::STRATIS_FILESYSTEM_NOTFOUND => "Filesystem not found",
            StratisErrorEnum::STRATIS_CACHE_NOTFOUND => "Cache not found",
            StratisErrorEnum::STRATIS_BAD_PARAM => "Bad parameter",
            StratisErrorEnum::STRATIS_DEV_NOTFOUND => "Dev not found",
            StratisErrorEnum::STRATIS_ALREADY_EXISTS => "Already exists",
            StratisErrorEnum::STRATIS_NULL_NAME => "Null name supplied",
            StratisErrorEnum::STRATIS_NO_POOLS => "No pools",
            StratisErrorEnum::STRATIS_LIST_FAILURE => "List operation failure.",
        }
    }
}

custom_derive! {
    #[derive(Copy, Clone, EnumDisplay,
             IterVariants(StratisDBusRaidTypeVariants),
             IterVariantNames(StratisDBusRaidTypeVariantNames))]
    #[allow(non_camel_case_types)]
    pub enum StratisRaidType {
        STRATIS_RAID_TYPE_UNKNOWN,
        STRATIS_RAID_TYPE_SINGLE,
        STRATIS_RAID_TYPE_RAID1,
        STRATIS_RAID_TYPE_RAID5,
        STRATIS_RAID_TYPE_RAID6,
    }
}

impl HasCodes for StratisRaidType {
    fn get_error_int(&self) -> u16 {
        *self as u16
    }

    fn get_error_string(&self) -> &str {
        match *self {
            StratisRaidType::STRATIS_RAID_TYPE_UNKNOWN => "Unknown",
            StratisRaidType::STRATIS_RAID_TYPE_SINGLE => "Single",
            StratisRaidType::STRATIS_RAID_TYPE_RAID1 => "Mirrored",
            StratisRaidType::STRATIS_RAID_TYPE_RAID5 => {
                "Block-level striping with distributed parity"
            }
            StratisRaidType::STRATIS_RAID_TYPE_RAID6 => {
                "Block-level striping with two distributed parities"
            }
        }
    }
}
