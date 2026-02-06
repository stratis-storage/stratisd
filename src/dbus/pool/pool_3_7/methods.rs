// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use crate::{
    dbus::{
        consts::OK_STRING,
        types::DbusErrorEnum,
        util::{engine_to_dbus_err_tuple, tuple_to_option},
    },
    engine::{Engine, PoolIdentifier, PoolUuid},
    stratis::StratisError,
};

pub async fn metadata_method(
    engine: &Arc<dyn Engine>,
    pool_uuid: PoolUuid,
    current: bool,
) -> (String, u16, String) {
    let default_return = String::new();

    let guard_res = engine
        .get_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    match tokio::task::spawn_blocking(move || {
        let guard = guard_res?;
        let (name, _, pool) = guard.as_tuple();
        if current {
            pool.current_metadata(&name)
        } else {
            pool.last_metadata()
        }
    })
    .await
    {
        Ok(Ok(metadata)) => (metadata, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Ok(Err(e)) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::from(e));
            (default_return, rc, rs)
        }
    }
}

pub async fn filesystem_metadata_method(
    engine: &Arc<dyn Engine>,
    pool_uuid: PoolUuid,
    fs_name: (bool, &str),
    current: bool,
) -> (String, u16, String) {
    let default_return = String::new();

    let fs_name_opt = tuple_to_option(fs_name).map(|f| f.to_owned());

    let guard_res = engine
        .get_pool(PoolIdentifier::Uuid(pool_uuid))
        .await
        .ok_or_else(|| StratisError::Msg(format!("No pool associated with uuid {pool_uuid}")));
    match tokio::task::spawn_blocking(move || {
        let guard = guard_res?;
        if current {
            guard.current_fs_metadata(fs_name_opt.as_deref())
        } else {
            guard.last_fs_metadata(fs_name_opt.as_deref())
        }
    })
    .await
    {
        Ok(Ok(metadata)) => (metadata, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Ok(Err(e)) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::from(e));
            (default_return, rc, rs)
        }
    }
}
