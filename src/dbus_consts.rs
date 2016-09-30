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
pub const STRATIS_VOLUME_BASE_INTERFACE: &'static str = "org.storage.stratis1.volume";
pub const STRATIS_DEV_BASE_INTERFACE: &'static str = "org.storage.stratis1.dev";
pub const STRATIS_CACHE_BASE_INTERFACE: &'static str = "org.storage.stratis1.cache";
pub const STRATIS_POOL_BASE_PATH: &'static str = "/org/storage/stratis/pool";


pub const LIST_POOLS: &'static str = "ListPools";
pub const CREATE_POOL: &'static str = "CreatePool";
pub const DESTROY_POOL: &'static str = "DestroyPool";
pub const GET_POOL_OBJECT_PATH: &'static str = "GetPoolObjectPath";
pub const GET_VOLUME_OBJECT_PATH: &'static str = "GetVolumeObjectPath";
pub const GET_DEV_OBJECT_PATH: &'static str = "GetDevObjectPath";
pub const GET_CACHE_OBJECT_PATH: &'static str = "GetCacheObjectPath";
pub const GET_ERROR_CODES: &'static str = "GetErrorCodes";
pub const GET_RAID_LEVELS: &'static str = "GetRaidLevels";
pub const GET_DEV_TYPES: &'static str = "GetDevTypes";

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
        STRATIS_NULL,
        STRATIS_NOTFOUND,
        STRATIS_POOL_NOTFOUND,
        STRATIS_VOLUME_NOTFOUND,
        STRATIS_DEV_NOTFOUND,
        STRATIS_CACHE_NOTFOUND,
        STRATIS_BAD_PARAM,
        STRATIS_ALREADY_EXISTS,
        STRATIS_NULL_NAME,
        STRATIS_NO_POOLS,
        STRATIS_LIST_FAILURE,
    }
}

impl HasCodes for StratisErrorEnum {
    fn get_error_int(&self) -> u16 {
        *self as u16
    }

    fn get_error_string(&self) -> &str {
        match *self {
            // TODO deal with internationalization/do this better
            StratisErrorEnum::STRATIS_OK => "Ok",
            StratisErrorEnum::STRATIS_ERROR => "A general error happened",
            StratisErrorEnum::STRATIS_NULL => "Null parameter was supplied",
            StratisErrorEnum::STRATIS_NOTFOUND => "Not found",
            StratisErrorEnum::STRATIS_POOL_NOTFOUND => "Pool not found",
            StratisErrorEnum::STRATIS_VOLUME_NOTFOUND => "Volume not found",
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
            StratisRaidType::STRATIS_RAID_TYPE_UNKNOWN => "Ok",
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
