
pub const DBUS_TIMEOUT: i32 = 20000; // millieconds
pub const SECTOR_SIZE: u64 = 512;



pub const STRATIS_VERSION: &'static str = "1";
pub const MANAGER_NAME:  &'static str = "/Manager";
pub const STRATIS_BASE_PATH:  &'static str = "/org/storage/stratis1";
pub const STRATIS_BASE_SERVICE:  &'static str = "org.storage.stratis1";
pub const STRATIS_BASE_MANAGER:  &'static str = "/org/storage/stratis1/Manager";
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


pub enum StratisErrorEnum {
    STRATIS_OK,
    STRATIS_ERROR,
    STRATIS_NULL,
    STRATIS_MALLOC,
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
    STRATIS_ERROR_MAX,
}


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
    STRATIS_RAID_TYPE_MAX,
}
