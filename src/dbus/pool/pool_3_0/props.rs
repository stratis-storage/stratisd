// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use zbus::{
    fdo::Error,
    zvariant::{OwnedValue, Str},
};

use crate::engine::{Engine, PoolIdentifier, PoolUuid};

pub fn uuid_prop(uuid: PoolUuid) -> String {
    uuid.to_string()
}

pub async fn name_prop(engine: &Arc<dyn Engine>, uuid: PoolUuid) -> Result<OwnedValue, Error> {
    let guard = engine
        .get_pool(PoolIdentifier::Uuid(uuid))
        .await
        .ok_or_else(|| Error::Failed(format!("No pool associated with UUID {uuid}")))?;
    let (name, _, _) = guard.as_tuple();
    Ok(OwnedValue::from(Str::from(name.to_string())))
}
