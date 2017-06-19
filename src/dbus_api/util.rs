// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::error::Error;

use dbus;
use dbus::MessageItem;
use dbus::arg::{ArgType, Iter, IterAppend};
use dbus::tree::{MethodErr, MTFn, PropInfo};

use engine::{EngineError, ErrorEnum};

use super::types::{DbusErrorEnum, TData};

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
    let value: T = try!(iter.read::<T>()
                            .map_err(|_| MethodErr::invalid_arg(&loc)));
    Ok(value)
}


/// Translates an engine error to a dbus error.
pub fn engine_to_dbus_err(err: &EngineError) -> (DbusErrorEnum, String) {
    let error = match *err {
        EngineError::Engine(ref e, _) => {
            match *e {
                ErrorEnum::Error | ErrorEnum::Invalid => DbusErrorEnum::ERROR,
                ErrorEnum::AlreadyExists => DbusErrorEnum::ALREADY_EXISTS,
                ErrorEnum::Busy => DbusErrorEnum::BUSY,
                ErrorEnum::NotFound => DbusErrorEnum::NOTFOUND,
            }
        }
        EngineError::Io(_) => DbusErrorEnum::IO_ERROR,
        EngineError::Nix(_) => DbusErrorEnum::NIX_ERROR,
        EngineError::Uuid(_) |
        EngineError::Utf8(_) |
        EngineError::Serde(_) |
        EngineError::DM(_) => DbusErrorEnum::INTERNAL_ERROR,
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
    dbus::Path::new("/").expect("'/' is guaranteed to be a valid Path.")
}

/// Similar to Option::ok_or, but unpacks a reference to a reference.
pub fn ref_ok_or<E, T>(opt: &Option<T>, err: E) -> Result<&T, E> {
    match *opt {
        Some(ref t) => Ok(t),
        None => Err(err),
    }
}

/// Get the UUID for an object path.
pub fn get_uuid(i: &mut IterAppend, p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    let object_path = p.path.get_name();
    let path = p.tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let data = try!(ref_ok_or(path.get_data(),
                              MethodErr::failed(&format!("no data for object path {}",
                                                         &object_path))));
    i.append(MessageItem::Str(format!("{}", data.uuid.simple())));
    Ok(())
}


/// Get the parent object path for an object path.
pub fn get_parent(i: &mut IterAppend, p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    let object_path = p.path.get_name();
    let path = p.tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let data = try!(ref_ok_or(path.get_data(),
                              MethodErr::failed(&format!("no data for object path {}",
                                                         &object_path))));
    i.append(MessageItem::ObjectPath(data.parent.clone()));
    Ok(())
}
