// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use crate::{
    dbus::types::ManagerR2,
    engine::{Engine, StoppedPoolsInfo},
};

pub async fn stopped_pools_prop(engine: &Arc<dyn Engine>) -> ManagerR2<StoppedPoolsInfo> {
    ManagerR2 {
        inner: engine.stopped_pools().await,
    }
}
