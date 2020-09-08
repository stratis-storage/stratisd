// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    arg::{ArgType, Iter, IterAppend, RefArg, Variant},
    ffidisp::{stdintf::org_freedesktop_dbus::PropertiesPropertiesChanged, Connection},
    message::SignalArgs,
    tree::{MTFn, MethodErr, PropInfo},
};

use devicemapper::DmError;

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, DbusErrorEnum, TData},
    },
    stratis::{ErrorEnum, StratisError},
};

/// Convert a tuple as option to an Option type
pub fn tuple_to_option<T>(value: (bool, T)) -> Option<T> {
    if value.0 {
        Some(value.1)
    } else {
        None
    }
}

/// Convert an option type to a tuple as option
pub fn option_to_tuple<T>(value: Option<T>, default: T) -> (bool, T) {
    match value {
        Some(v) => (true, v),
        None => (false, default),
    }
}

/// Map a result obtained for the FetchProperties interface to a value used
/// to represent an option.  An error in the result
/// argument yields a false in the return value, indicating that the value
/// returned is a string representation of the error encountered in
/// obtaining the value, and not the value requested.
pub fn result_to_tuple<T>(result: Result<T, String>) -> (bool, Variant<Box<dyn RefArg>>)
where
    T: RefArg + 'static,
{
    let (success, value) = match result {
        Ok(value) => (true, Variant(Box::new(value) as Box<dyn RefArg>)),
        Err(e) => (false, Variant(Box::new(e) as Box<dyn RefArg>)),
    };
    (success, value)
}

/// Get the next argument off the bus. loc is the index of the location of
/// the argument in the iterator, and is used solely for error-reporting.
pub fn get_next_arg<'a, T>(iter: &mut Iter<'a>, loc: u16) -> Result<T, MethodErr>
where
    T: dbus::arg::Get<'a> + dbus::arg::Arg,
{
    if iter.arg_type() == ArgType::Invalid {
        return Err(MethodErr::no_arg());
    };
    let value: T = iter.read::<T>().map_err(|_| MethodErr::invalid_arg(&loc))?;
    Ok(value)
}

/// Generate a new object path which is guaranteed unique wrt. all previously
/// generated object paths.
pub fn make_object_path(context: &DbusContext) -> String {
    format!(
        "{}/{}",
        consts::STRATIS_BASE_PATH,
        context.get_next_id().to_string()
    )
}

/// Translates an engine error to the (errorcode, string) tuple that Stratis
/// D-Bus methods return.
pub fn engine_to_dbus_err_tuple(err: &StratisError) -> (u16, String) {
    let error = match *err {
        StratisError::Error(_) => DbusErrorEnum::ERROR,
        StratisError::Engine(ref e, _) => match *e {
            ErrorEnum::Error => DbusErrorEnum::ERROR,
            ErrorEnum::AlreadyExists => DbusErrorEnum::ALREADY_EXISTS,
            ErrorEnum::Busy => DbusErrorEnum::BUSY,
            ErrorEnum::Invalid => DbusErrorEnum::ERROR,
            ErrorEnum::NotFound => DbusErrorEnum::NOTFOUND,
        },
        StratisError::Io(_) => DbusErrorEnum::ERROR,
        StratisError::Nix(_) => DbusErrorEnum::ERROR,
        StratisError::Uuid(_)
        | StratisError::Utf8(_)
        | StratisError::Serde(_)
        | StratisError::DM(_)
        | StratisError::Dbus(_)
        | StratisError::Udev(_)
        | StratisError::Crypt(_) => DbusErrorEnum::ERROR,
    };
    let description = match *err {
        StratisError::DM(DmError::Core(ref err)) => err.to_string(),
        ref err => err.to_string(),
    };
    (error as u16, description)
}

/// Convenience function to get the error value for "OK"
pub fn msg_code_ok() -> u16 {
    DbusErrorEnum::OK as u16
}

/// Convenience function to get the error string for "OK"
pub fn msg_string_ok() -> String {
    DbusErrorEnum::OK.get_error_string().to_owned()
}

/// Get the UUID for an object path.
pub fn get_uuid(i: &mut IterAppend, p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    let object_path = p.path.get_name();
    let path = p
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");

    let data = path
        .get_data()
        .as_ref()
        .ok_or_else(|| MethodErr::failed(&format!("no data for object path {}", object_path)))?;

    i.append(uuid_to_string!(data.uuid));
    Ok(())
}

/// Get the parent object path for an object path.
pub fn get_parent(i: &mut IterAppend, p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    let object_path = p.path.get_name();
    let path = p
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");

    let data = path
        .get_data()
        .as_ref()
        .ok_or_else(|| MethodErr::failed(&format!("no data for object path {}", object_path)))?;

    i.append(data.parent.clone());
    Ok(())
}

/// Place a property changed signal on the D-Bus for the given property name
/// and value and for all interfaces specified.
pub fn prop_changed_dispatch<T: 'static>(
    conn: &Connection,
    prop_name: &str,
    new_value: T,
    path: &dbus::Path,
    interfaces: &[String],
) -> Result<(), ()>
where
    T: RefArg,
{
    let mut prop_changed: PropertiesPropertiesChanged = Default::default();
    prop_changed
        .changed_properties
        .insert(prop_name.into(), Variant(Box::new(new_value)));

    for interface in interfaces {
        prop_changed.interface_name = interface.to_owned();
        conn.send(prop_changed.to_emit_message(path))?;
    }

    Ok(())
}
