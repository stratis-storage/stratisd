// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use zbus::fdo::Error;

use crate::engine::{Engine, Name, PoolIdentifier, PoolUuid};

pub async fn name_prop(engine: &Arc<dyn Engine>, uuid: PoolUuid) -> Result<Name, Error> {
    let guard = engine
        .get_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| Error::Failed(format!("No pool associated with UUID {uuid}")))?;
    let (name, _, _) = guard.as_tuple();
    Ok(name)
}
