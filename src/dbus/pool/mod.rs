// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use tokio::sync::RwLock;
use zbus::{
    zvariant::{ObjectPath, OwnedObjectPath},
    Connection,
};

use crate::{
    dbus::{consts, Manager},
    engine::{Engine, Lockable, PoolUuid},
    stratis::StratisResult,
};

mod pool_3_0;
mod pool_3_9;
mod shared;

pub use pool_3_9::PoolR9;

pub async fn register_pool<'a>(
    manager: &Lockable<Arc<RwLock<Manager>>>,
    connection: &Arc<Connection>,
    counter: &Arc<AtomicU64>,
    engine: Arc<dyn Engine>,
    pool_uuid: PoolUuid,
) -> StratisResult<(ObjectPath<'a>, Vec<ObjectPath<'a>>)> {
    let path = ObjectPath::try_from(format!(
        "{}/{}",
        consts::STRATIS_BASE_PATH,
        counter.fetch_add(1, Ordering::AcqRel),
    ))?;
    PoolR9::register(connection, path.clone(), engine, pool_uuid).await?;

    manager
        .write()
        .await
        .add_pool(pool_uuid, OwnedObjectPath::from(path.clone()));

    Ok((path, Vec::default()))
}
