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
pub const CONFIGURE_SIMULATOR: &'static str = "ConfigureSimulator";

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

// Filesystem Methods
pub const CREATE_SNAPSHOT: &'static str = "CreateSnapshot";
pub const RENAME_FILESYSTEM: &'static str = "Rename";
pub const SET_MOUNTPOINT: &'static str = "SetMountpoint";
pub const SET_QUOTA: &'static str = "SetQuota";

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
    pub enum ErrorEnum {
        OK,
        ERROR,

        ALREADY_EXISTS,
        BAD_PARAM,
        BUSY,
        CACHE_NOTFOUND,
        DEV_NOTFOUND,
        FILESYSTEM_NOTFOUND,
        IO_ERROR,
        LIST_FAILURE,
        INTERNAL_ERROR,
        NIX_ERROR,
        NO_POOLS,
        NOTFOUND,
        NULL,
        NULL_NAME,
        POOL_NOTFOUND,
    }
}

impl HasCodes for ErrorEnum {
    fn get_error_int(&self) -> u16 {
        *self as u16
    }

    fn get_error_string(&self) -> &str {
        match *self {
            ErrorEnum::OK => "Ok",
            ErrorEnum::ERROR => "A general error happened",
            ErrorEnum::NULL => "Null parameter was supplied",
            ErrorEnum::NOTFOUND => "Not found",
            ErrorEnum::POOL_NOTFOUND => "Pool not found",
            ErrorEnum::FILESYSTEM_NOTFOUND => "Filesystem not found",
            ErrorEnum::CACHE_NOTFOUND => "Cache not found",
            ErrorEnum::BAD_PARAM => "Bad parameter",
            ErrorEnum::DEV_NOTFOUND => "Dev not found",
            ErrorEnum::ALREADY_EXISTS => "Already exists",
            ErrorEnum::NULL_NAME => "Null name supplied",
            ErrorEnum::NO_POOLS => "No pools",
            ErrorEnum::LIST_FAILURE => "List operation failure.",
            ErrorEnum::INTERNAL_ERROR => "Internal error",
            ErrorEnum::IO_ERROR => "IO error during operation.",
            ErrorEnum::NIX_ERROR => "System error during operation.",
            ErrorEnum::BUSY => "Operation can not be performed at this time",
        }
    }
}

custom_derive! {
    #[derive(Copy, Clone, EnumDisplay,
             IterVariants(StratisDBusRaidTypeVariants),
             IterVariantNames(StratisDBusRaidTypeVariantNames))]
    #[allow(non_camel_case_types)]
    pub enum RaidType {
        RAID_TYPE_UNKNOWN,
        RAID_TYPE_SINGLE,
        RAID_TYPE_RAID1,
        RAID_TYPE_RAID5,
        RAID_TYPE_RAID6,
    }
}

impl HasCodes for RaidType {
    fn get_error_int(&self) -> u16 {
        *self as u16
    }

    fn get_error_string(&self) -> &str {
        match *self {
            RaidType::RAID_TYPE_UNKNOWN => "Unknown",
            RaidType::RAID_TYPE_SINGLE => "Single",
            RaidType::RAID_TYPE_RAID1 => "Mirrored",
            RaidType::RAID_TYPE_RAID5 => "Block-level striping with distributed parity",
            RaidType::RAID_TYPE_RAID6 => "Block-level striping with two distributed parities",
        }
    }
}
