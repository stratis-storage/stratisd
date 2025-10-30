// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::Bytes;

use crate::{
    dbus::util::option_to_tuple,
    engine::{Name, Pool, PoolUuid, SomeLockReadGuard},
};

pub fn name_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Name {
    let (name, _, _) = guard.as_tuple();
    name
}

pub fn size_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> String {
    (*guard.total_physical_size()).to_string()
}

pub fn used_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> (bool, String) {
    let (b, used) = option_to_tuple(
        guard.total_physical_used().map(|s| s.bytes()),
        Bytes::from(0u64),
    );
    (b, (*used).to_string())
}

pub fn allocated_prop(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> String {
    (*guard.total_allocated_size()).to_string()
}
