// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use zbus::{zvariant::ObjectPath, Connection};

use crate::{
    dbus::consts,
    engine::{Engine, PoolUuid},
    stratis::StratisResult,
};

mod pool_3_0;
mod pool_3_9;

pub use pool_3_9::PoolR9;

pub async fn register_pool<'a>(
    engine: &Arc<dyn Engine>,
    connection: &Arc<Connection>,
    counter: &Arc<AtomicU64>,
    pool_uuid: PoolUuid,
) -> StratisResult<(ObjectPath<'a>, Vec<ObjectPath<'a>>)> {
    PoolR9::register(
        engine,
        connection,
        ObjectPath::try_from(format!(
            "{}/{}",
            consts::STRATIS_BASE_PATH,
            counter.fetch_add(1, Ordering::AcqRel),
        ))?,
        pool_uuid,
    )
    .await?;

    Ok((ObjectPath::default(), Vec::default()))
}
