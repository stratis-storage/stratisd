// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::engine::{Pool, PoolUuid, SomeLockReadGuard};

pub fn metadata_used_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> String {
    (*guard.metadata_used().bytes()).to_string()
}
