// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, fmt::Display, future::Future, sync::Arc};

use dbus::{
    arg::{ArgType, Iter, IterAppend, RefArg, Variant},
    blocking::SyncConnection,
};
use dbus_tree::{MTSync, MethodErr, PropInfo};
use futures::{
    executor::block_on,
    future::{select, Either},
    pin_mut,
};
use tokio::sync::{
    broadcast::{error::RecvError, Sender},
    mpsc::{UnboundedReceiver, UnboundedSender},
};

use devicemapper::DmError;

use crate::{
    dbus_api::{
        api::get_base_tree,
        connection::DbusConnectionHandler,
        consts,
        tree::DbusTreeHandler,
        types::{
            DbusAction, DbusContext, DbusErrorEnum, DbusHandlers, InterfacesAdded,
            InterfacesAddedThreadSafe, TData,
        },
        udev::DbusUdevHandler,
    },
    engine::{Engine, Lockable, LockableEngine, UdevEngineEvent},
    stratis::{StratisError, StratisResult},
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

/// Map a result containing an option obtained for the FetchProperties interface to
/// a value used to represent both the result and option.  An error in the result
/// argument yields a false in the return value, indicating that the value
/// returned is a string representation of the error encountered in
/// obtaining the value, and not the value requested. If the first boolean is true,
/// the variant will be a tuple of type (bool, T). If the second boolean if false,
/// this indicates None. If it is true, the value for T is the Some(_) value.
pub fn result_option_to_tuple<T, E>(
    result: Result<Option<T>, E>,
    default: T,
) -> (bool, Variant<Box<dyn RefArg>>)
where
    T: RefArg + 'static,
    E: Display,
{
    let (success, value) = match result {
        Ok(value) => (
            true,
            Variant(Box::new(option_to_tuple(value, default)) as Box<dyn RefArg>),
        ),
        Err(e) => (false, Variant(Box::new(e.to_string()) as Box<dyn RefArg>)),
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
pub fn make_object_path<E>(context: &DbusContext<E>) -> String
where
    E: Engine,
{
    format!(
        "{}/{}",
        consts::STRATIS_BASE_PATH,
        context.get_next_id().to_string()
    )
}

/// Translates an engine error to the (errorcode, string) tuple that Stratis
/// D-Bus methods return.
pub fn engine_to_dbus_err_tuple(err: &StratisError) -> (u16, String) {
    let description = match *err {
        StratisError::DM(DmError::Core(ref err)) => err.to_string(),
        ref err => err.to_string(),
    };
    (DbusErrorEnum::ERROR as u16, description)
}

/// Get the UUID for an object path.
pub fn get_uuid<E>(
    i: &mut IterAppend,
    p: &PropInfo<MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: Engine,
{
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
pub fn get_parent<E>(
    i: &mut IterAppend,
    p: &PropInfo<MTSync<TData<E>>, TData<E>>,
) -> Result<(), MethodErr>
where
    E: Engine,
{
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

/// Create both ends of the D-Bus processing handlers.
/// Returns a triple:
/// 1. A DbusConnectionHandler which may be used to process D-Bus methods calls
/// 2. A DbusUdevHandler which may be used to handle detected udev events
/// 3. A DbusTreeHandler which may be used to update the D-Bus tree
///
/// Messages may be:
/// * received by the DbusUdevHandler from the udev thread,
/// * sent by the DbusContext to the DbusTreeHandler
pub fn create_dbus_handlers<E>(
    engine: LockableEngine<E>,
    udev_receiver: UnboundedReceiver<UdevEngineEvent>,
    trigger: Sender<()>,
    (tree_sender, tree_receiver): (
        UnboundedSender<DbusAction<E>>,
        UnboundedReceiver<DbusAction<E>>,
    ),
) -> DbusHandlers<E>
where
    E: 'static + Engine,
{
    let conn = Arc::new(SyncConnection::new_system()?);
    let (tree, object_path) =
        get_base_tree(DbusContext::new(engine, tree_sender, Arc::clone(&conn)));
    let dbus_context = tree.get_data().clone();
    conn.request_name(consts::STRATIS_BASE_SERVICE, false, true, true)?;

    let tree = Lockable::new_shared(tree);
    let connection =
        DbusConnectionHandler::new(Arc::clone(&conn), tree.clone(), trigger.subscribe());
    let udev = DbusUdevHandler::new(udev_receiver, object_path, dbus_context);
    let tree = DbusTreeHandler::new(tree, tree_receiver, conn, trigger.subscribe());
    Ok((connection, udev, tree))
}

/// This method converts the thread safe representation of D-Bus property maps to a type
/// that can be sent over the D-Bus.
pub fn thread_safe_to_dbus_sendable(ia: InterfacesAddedThreadSafe) -> InterfacesAdded {
    ia.into_iter()
        .map(|(k, map)| {
            let new_map: HashMap<String, Variant<Box<dyn RefArg>>> = map
                .into_iter()
                .map(|(subk, var)| (subk, Variant(var.0 as Box<dyn RefArg>)))
                .collect();
            (k, new_map)
        })
        .collect()
}

pub fn poll_exit_and_future<E, F, R>(exit: E, future: F) -> StratisResult<Option<R>>
where
    E: Future<Output = Result<(), RecvError>>,
    F: Future<Output = R>,
{
    pin_mut!(exit);
    pin_mut!(future);

    match block_on(select(exit, future)) {
        Either::Left((Ok(()), _)) => {
            info!("D-Bus tree handler was notified to exit");
            Ok(None)
        }
        Either::Left((Err(_), _)) => Err(StratisError::Msg(
            "D-Bus tree handler can no longer be notified to exit; shutting down...".to_string(),
        )),
        Either::Right((a, _)) => Ok(Some(a)),
    }
}
