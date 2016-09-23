
use std::fmt;
use std::slice::Iter;

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

#[derive(Copy, Clone)]
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

impl StratisErrorEnum {
    pub fn iterator() -> Iter<'static, StratisErrorEnum> {
        static CODES: [StratisErrorEnum; 13] = [StratisErrorEnum::STRATIS_OK,
                                                StratisErrorEnum::STRATIS_ERROR,
                                                StratisErrorEnum::STRATIS_NULL,
                                                StratisErrorEnum::STRATIS_NOTFOUND,
                                                StratisErrorEnum::STRATIS_POOL_NOTFOUND,
                                                StratisErrorEnum::STRATIS_VOLUME_NOTFOUND,
                                                StratisErrorEnum::STRATIS_DEV_NOTFOUND,
                                                StratisErrorEnum::STRATIS_CACHE_NOTFOUND,
                                                StratisErrorEnum::STRATIS_BAD_PARAM,
                                                StratisErrorEnum::STRATIS_ALREADY_EXISTS,
                                                StratisErrorEnum::STRATIS_NULL_NAME,
                                                StratisErrorEnum::STRATIS_NO_POOLS,
                                                StratisErrorEnum::STRATIS_LIST_FAILURE];
        CODES.into_iter()
    }

    pub fn get_error_int(error: &StratisErrorEnum) -> u16 {
        *error as u16
    }

    pub fn get_error_string(error: &StratisErrorEnum) -> &str {
        match *error {
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
impl fmt::Display for StratisErrorEnum {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            StratisErrorEnum::STRATIS_OK => write!(f, "STRATIS_OK"),
            StratisErrorEnum::STRATIS_ERROR => write!(f, "STRATIS_ERROR"),
            StratisErrorEnum::STRATIS_NULL => write!(f, "STRATIS_NULL"),
            StratisErrorEnum::STRATIS_NOTFOUND => write!(f, "STRATIS_NOTFOUND"),
            StratisErrorEnum::STRATIS_POOL_NOTFOUND => write!(f, "STRATIS_POOL_NOTFOUND"),
            StratisErrorEnum::STRATIS_VOLUME_NOTFOUND => write!(f, "STRATIS_VOLUME_NOTFOUND"),
            StratisErrorEnum::STRATIS_CACHE_NOTFOUND => write!(f, "STRATIS_CACHE_NOTFOUND"),
            StratisErrorEnum::STRATIS_BAD_PARAM => write!(f, "STRATIS_BAD_PARAM"),
            StratisErrorEnum::STRATIS_DEV_NOTFOUND => write!(f, "STRATIS_DEV_NOTFOUND"),
            StratisErrorEnum::STRATIS_ALREADY_EXISTS => write!(f, "STRATIS_ALREADY_EXISTS"),
            StratisErrorEnum::STRATIS_NULL_NAME => write!(f, "STRATIS_NULL_NAME"),
            StratisErrorEnum::STRATIS_NO_POOLS => write!(f, "STRATIS_NO_POOLS"),
            StratisErrorEnum::STRATIS_LIST_FAILURE => write!(f, "STRATIS_LIST_FAILURE"),
        }
    }
}
#[derive(Copy, Clone)]
#[allow(non_camel_case_types)]
pub enum StratisRaidType {
    STRATIS_RAID_TYPE_UNKNOWN,
    /** Single */
    STRATIS_RAID_TYPE_SINGLE,
    /** Mirror between two disks. For 4 disks or more, they are RAID10.*/
    STRATIS_RAID_TYPE_RAID1,
    /** Block-level striping with distributed parity */
    STRATIS_RAID_TYPE_RAID5,
    /** Block-level striping with two distributed parities, aka, RAID-DP */
    STRATIS_RAID_TYPE_RAID6,
}

impl StratisRaidType {
    pub fn iterator() -> Iter<'static, StratisRaidType> {
        static TYPES: [StratisRaidType; 5] = [StratisRaidType::STRATIS_RAID_TYPE_UNKNOWN,
                                              StratisRaidType::STRATIS_RAID_TYPE_SINGLE,
                                              StratisRaidType::STRATIS_RAID_TYPE_RAID1,
                                              StratisRaidType::STRATIS_RAID_TYPE_RAID5,
                                              StratisRaidType::STRATIS_RAID_TYPE_RAID6];
        TYPES.into_iter()
    }

    pub fn get_error_int(error: &StratisRaidType) -> u16 {
        *error as u16
    }

    pub fn get_error_string(error: &StratisRaidType) -> &str {
        match *error {
            // TODO deal with internationalization/do this better
            StratisRaidType::STRATIS_RAID_TYPE_UNKNOWN => "Ok",
            StratisRaidType::STRATIS_RAID_TYPE_SINGLE => "Single",
            StratisRaidType::STRATIS_RAID_TYPE_RAID1 => {
                "Mirror between two disks. For 4 disks or more, they are RAID10"
            }
            StratisRaidType::STRATIS_RAID_TYPE_RAID5 => {
                "Block-level striping with distributed parity"
            }
            StratisRaidType::STRATIS_RAID_TYPE_RAID6 => {
                "Block-level striping with two distributed parities, aka, RAID-DP"
            }
        }
    }
}
impl fmt::Display for StratisRaidType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            StratisRaidType::STRATIS_RAID_TYPE_UNKNOWN => write!(f, "STRATIS_RAID_TYPE_UNKNOWN"),
            StratisRaidType::STRATIS_RAID_TYPE_SINGLE => write!(f, "STRATIS_RAID_TYPE_SINGLE"),
            StratisRaidType::STRATIS_RAID_TYPE_RAID1 => write!(f, "STRATIS_RAID_TYPE_RAID1"),
            StratisRaidType::STRATIS_RAID_TYPE_RAID5 => write!(f, "STRATIS_RAID_TYPE_RAID5"),
            StratisRaidType::STRATIS_RAID_TYPE_RAID6 => write!(f, "STRATIS_RAID_TYPE_RAID6"),
        }
    }
}
