// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

custom_derive! {
    #[derive(Copy, Clone, EnumDisplay,
             IterVariants(StratisDBusErrorVariants),
             IterVariantNames(StratisDBusErrorVariantNames))]
    #[allow(non_camel_case_types)]
    pub enum DbusErrorEnum {
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

/// Get the u16 value of this ErrorEnum constructor.
impl From<DbusErrorEnum> for u16 {
    fn from(e: DbusErrorEnum) -> u16 {
        e as u16
    }
}

impl DbusErrorEnum {
    pub fn get_error_string(&self) -> &str {
        match *self {
            DbusErrorEnum::OK => "Ok",
            DbusErrorEnum::ERROR => "A general error happened",
            DbusErrorEnum::NULL => "Null parameter was supplied",
            DbusErrorEnum::NOTFOUND => "Not found",
            DbusErrorEnum::POOL_NOTFOUND => "Pool not found",
            DbusErrorEnum::FILESYSTEM_NOTFOUND => "Filesystem not found",
            DbusErrorEnum::CACHE_NOTFOUND => "Cache not found",
            DbusErrorEnum::BAD_PARAM => "Bad parameter",
            DbusErrorEnum::DEV_NOTFOUND => "Dev not found",
            DbusErrorEnum::ALREADY_EXISTS => "Already exists",
            DbusErrorEnum::NULL_NAME => "Null name supplied",
            DbusErrorEnum::NO_POOLS => "No pools",
            DbusErrorEnum::LIST_FAILURE => "List operation failure",
            DbusErrorEnum::INTERNAL_ERROR => "Internal error",
            DbusErrorEnum::IO_ERROR => "IO error during operation",
            DbusErrorEnum::NIX_ERROR => "System error during operation",
            DbusErrorEnum::BUSY => "Operation can not be performed at this time",
        }
    }
}
