// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

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

/// Get the u16 value of this ErrorEnum constructor.
impl From<ErrorEnum> for u16 {
    fn from(e: ErrorEnum) -> u16 {
        e as u16
    }
}

impl ErrorEnum {
    pub fn get_error_string(&self) -> &str {
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
