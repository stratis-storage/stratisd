// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::SecondsFormat;
use zbus::zvariant::Structure;

use crate::{
    dbus::util::option_to_tuple,
    engine::{Pool, PoolUuid, SomeLockReadGuard},
};

pub fn last_reencrypted_timestamp_prop<'a>(
    guard: SomeLockReadGuard<PoolUuid, dyn Pool>,
) -> Structure<'a> {
    Structure::from(option_to_tuple(
        guard
            .last_reencrypt()
            .map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true)),
        String::default(),
    ))
}
