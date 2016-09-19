
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
    Error,
    Null,
    Malloc,
    Notfound,
    PoolNotfound,
    VolumeNotfound,
    DevNotfound,
    CacheNotfound,
    BadParam,
    AlreadyExists,
    NullName,
    NoPools,
    ListFailure,
}


pub enum StratisRaidType {
    /** Single */
    Single,
    /** Mirror between two disks. For 4 disks or more, they are RAID10.*/
    Raid1,
    /** Block-level striping with distributed parity */
    Raid5,
    /** Block-level striping with two distributed parities, aka, RAID-DP */
    Raid6,
}
