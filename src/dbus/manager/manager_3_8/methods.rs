// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

use serde_json::{from_str, Value};
use zbus::{zvariant::ObjectPath, Connection};

use devicemapper::Bytes;

use crate::{
    dbus::{
        consts::OK_STRING,
        pool::register_pool,
        types::DbusErrorEnum,
        util::{engine_to_dbus_err_tuple, tuple_to_option},
    },
    engine::{
        CreateAction, Engine, InputEncryptionInfo, IntegritySpec, IntegrityTagSpec, KeyDescription,
    },
    stratis::{StratisError, StratisResult},
};

#[allow(clippy::too_many_arguments)]
pub async fn create_pool_method<'a>(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    counter: &Arc<AtomicU64>,
    name: &str,
    devs: Vec<PathBuf>,
    key_desc: Vec<((bool, u32), KeyDescription)>,
    clevis_info: Vec<((bool, u32), &str, &str)>,
    journal_size: (bool, u64),
    tag_spec: (bool, &str),
    allocate_superblock: (bool, bool),
) -> ((bool, (ObjectPath<'a>, Vec<ObjectPath<'a>>)), u16, String) {
    let default_return = (false, (ObjectPath::default(), Vec::new()));

    let devs_ref = devs.iter().map(|path| path.as_path()).collect::<Vec<_>>();
    let key_desc = key_desc
        .into_iter()
        .map(|(tup, kd)| (tuple_to_option(tup), kd))
        .collect::<Vec<_>>();
    let clevis_info = match clevis_info.into_iter().try_fold::<_, _, StratisResult<_>>(
        Vec::new(),
        |mut vec, (tup, s, json)| {
            vec.push((
                tuple_to_option(tup),
                (
                    s.to_string(),
                    from_str::<Value>(json).map_err(StratisError::from)?,
                ),
            ));
            Ok(vec)
        },
    ) {
        Ok(ci) => ci,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return (default_return, rc, rs);
        }
    };
    let iei = match InputEncryptionInfo::new(key_desc, clevis_info) {
        Ok(info) => info,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return (default_return, rc, rs);
        }
    };
    let journal_size = tuple_to_option(journal_size).map(Bytes::from);
    let tag_spec = match tuple_to_option(tag_spec)
        .map(IntegrityTagSpec::try_from)
        .transpose()
    {
        Ok(s) => s,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Msg(format!(
                "Failed to parse integrity tag specification: {e}"
            )));
            return (default_return, rc, rs);
        }
    };
    let allocate_superblock = tuple_to_option(allocate_superblock);

    match handle_action!(
        engine
            .create_pool(
                name,
                devs_ref.as_slice(),
                iei.as_ref(),
                IntegritySpec {
                    journal_size,
                    tag_spec,
                    allocate_superblock,
                },
            )
            .await
    ) {
        Ok(CreateAction::Created(uuid)) => {
            match register_pool(engine, connection, counter, uuid).await {
                Ok(tuple) => (
                    (true, tuple),
                    DbusErrorEnum::OK as u16,
                    OK_STRING.to_string(),
                ),
                Err(e) => {
                    let (rc, rs) = engine_to_dbus_err_tuple(&e);
                    (default_return, rc, rs)
                }
            }
        }
        Ok(CreateAction::Identity) => (
            default_return,
            DbusErrorEnum::OK as u16,
            OK_STRING.to_string(),
        ),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            (default_return, rc, rs)
        }
    }
}
