// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::error::Error;

use dbus;
use dbus::MessageItem;
use dbus::arg::{ArgType, Iter};
use dbus::tree::MethodErr;

use engine;
use engine::EngineError;

use super::types::DbusErrorEnum;

pub const STRATIS_BASE_PATH: &'static str = "/org/storage/stratis1";
pub const STRATIS_BASE_SERVICE: &'static str = "org.storage.stratis1";

/// Convert a tuple as option to an Option type
pub fn tuple_to_option<T>(value: (bool, T)) -> Option<T> {
    if value.0 { Some(value.1) } else { None }
}

/// Get the next argument off the bus
pub fn get_next_arg<'a, T>(iter: &mut Iter<'a>, loc: u16) -> Result<T, MethodErr>
    where T: dbus::arg::Get<'a> + dbus::arg::Arg
{
    if iter.arg_type() == ArgType::Invalid {
        return Err(MethodErr::no_arg());
    };
    let value: T = try!(iter.read::<T>().map_err(|_| MethodErr::invalid_arg(&loc)));
    Ok(value)
}


/// Translates an engine error to a dbus error.
pub fn engine_to_dbus_err(err: &EngineError) -> (DbusErrorEnum, String) {
    let error = match *err {
        EngineError::Engine(ref e, _) => {
            match *e {
                engine::ErrorEnum::Error => DbusErrorEnum::ERROR,
                engine::ErrorEnum::AlreadyExists => DbusErrorEnum::ALREADY_EXISTS,
                engine::ErrorEnum::Busy => DbusErrorEnum::BUSY,
                engine::ErrorEnum::Invalid => DbusErrorEnum::ERROR,
                engine::ErrorEnum::NotFound => DbusErrorEnum::NOTFOUND,
                engine::ErrorEnum::FailedToOpen => DbusErrorEnum::ERROR,
            }
        }
        EngineError::Io(_) => DbusErrorEnum::IO_ERROR,
        EngineError::Nix(_) => DbusErrorEnum::NIX_ERROR,
        EngineError::Uuid(_) => DbusErrorEnum::INTERNAL_ERROR,
        EngineError::Utf8(_) => DbusErrorEnum::INTERNAL_ERROR,
        EngineError::Serde(_) => DbusErrorEnum::INTERNAL_ERROR,
    };
    (error, err.description().to_owned())
}

/// Convenience function to convert a return code and a string to
/// appropriately typed MessageItems.
pub fn code_to_message_items(code: DbusErrorEnum, mes: String) -> (MessageItem, MessageItem) {
    (MessageItem::UInt16(code.into()), MessageItem::Str(mes))
}

/// Convenience function to directly yield MessageItems for OK code and message.
pub fn ok_message_items() -> (MessageItem, MessageItem) {
    let code = DbusErrorEnum::OK;
    code_to_message_items(code, code.get_error_string().into())
}

pub fn default_object_path<'a>() -> dbus::Path<'a> {
    // "/" is a valid path, so Path::new() must succeed.
    dbus::Path::new("/").unwrap()
}
