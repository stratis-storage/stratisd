// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, fmt::Display, future::Future, sync::Arc, time::Duration};

use dbus::arg::{ArgType, Iter, IterAppend, RefArg, Variant};
use dbus_tokio::connection::{new_system_sync, IOResourceError};
use dbus_tree::{MTSync, MethodErr, PropInfo};
use futures::{
    future::{select, Either},
    pin_mut,
};
use tokio::{
    sync::{
        broadcast::{error::RecvError, Sender},
        mpsc::{UnboundedReceiver, UnboundedSender},
    },
    task::JoinHandle,
    time::sleep,
};

use devicemapper::DmError;

use crate::{
    dbus_api::{
        api::get_base_tree,
        consts,
        message::DbusMessageHandler,
        tree::DbusTreeHandler,
        types::{
            DbusAction, DbusContext, DbusErrorEnum, DbusHandlers, InterfacesAdded,
            InterfacesAddedThreadSafe, TData,
        },
        udev::DbusUdevHandler,
    },
    engine::{Engine, Lockable, UdevEngineEvent},
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
pub fn make_object_path(context: &DbusContext) -> String {
    format!("{}/{}", consts::STRATIS_BASE_PATH, context.get_next_id())
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
pub fn get_uuid(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    let object_path = p.path.get_name();
    let path = p
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");

    let data = path
        .get_data()
        .as_ref()
        .ok_or_else(|| MethodErr::failed(&format!("no data for object path {object_path}")))?;

    i.append(uuid_to_string!(data.uuid));
    Ok(())
}

/// Get the parent object path for an object path.
pub fn get_parent(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    let object_path = p.path.get_name();
    let path = p
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");

    let data = path
        .get_data()
        .as_ref()
        .ok_or_else(|| MethodErr::failed(&format!("no data for object path {object_path}")))?;

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
pub async fn create_dbus_handlers(
    engine: Arc<dyn Engine>,
    udev_receiver: UnboundedReceiver<UdevEngineEvent>,
    trigger: Sender<()>,
    (tree_sender, tree_receiver): (UnboundedSender<DbusAction>, UnboundedReceiver<DbusAction>),
) -> DbusHandlers {
    let (resource, arc_conn) = spawn_blocking!(new_system_sync())??;
    let conn = Lockable::new_shared(arc_conn);
    let cloned_conn = conn.clone();
    let connection: JoinHandle<StratisResult<()>> = tokio::spawn(async move {
        let mut resource: JoinHandle<Result<(), IOResourceError>> =
            tokio::spawn(async { Err(resource.await) });
        cloned_conn
            .read()
            .await
            .request_name(consts::STRATIS_BASE_SERVICE, false, true, true)
            .await?;

        loop {
            let err = resource.await;
            match err {
                Ok(Err(e)) => {
                    warn!("Lost connection to D-Bus: {e}");
                }
                Err(e) => {
                    warn!("Error joining thread: {e}");
                }
                _ => unreachable!(),
            }
            loop {
                match spawn_blocking!(new_system_sync()) {
                    Ok(Ok((new_resource, arc_conn))) => {
                        *cloned_conn.write().await = arc_conn;
                        cloned_conn
                            .read()
                            .await
                            .request_name(consts::STRATIS_BASE_SERVICE, false, true, true)
                            .await?;
                        resource = tokio::spawn(async { Err(new_resource.await) });
                        break;
                    }
                    _ => sleep(Duration::from_secs(1)).await,
                };
            }
        }
    });

    let (tree, object_path) = get_base_tree(DbusContext::new(engine, tree_sender, conn.clone()));
    let dbus_context = tree.get_data().clone();
    let tree = Lockable::new_shared(tree);
    let message = DbusMessageHandler::new(conn.clone(), tree.clone());
    let udev = DbusUdevHandler::new(udev_receiver, object_path, dbus_context);
    let tree = DbusTreeHandler::new(tree, tree_receiver, conn.clone(), trigger.subscribe());
    Ok((message, connection, udev, tree))
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

pub async fn poll_exit_and_future<E, F, R>(exit: E, future: F) -> StratisResult<Option<R>>
where
    E: Future<Output = Result<(), RecvError>>,
    F: Future<Output = R>,
{
    pin_mut!(exit);
    pin_mut!(future);

    match select(exit, future).await {
        Either::Left((Ok(()), _)) => {
            info!("D-Bus tree handler was notified to exit");
            Ok(None)
        }
        Either::Left((Err(_), _)) => Err(StratisError::Msg(
            "Checking the shutdown signal failed so stratisd can no longer be notified to shut down; exiting now...".to_string(),
        )),
        Either::Right((a, _)) => Ok(Some(a)),
    }
}
