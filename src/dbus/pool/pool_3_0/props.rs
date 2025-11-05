// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use zbus::zvariant::{Str, Structure};

use devicemapper::Bytes;

use crate::{
    dbus::util::option_to_tuple,
    engine::{Pool, PoolUuid, SomeLockReadGuard},
};

pub fn uuid_prop(uuid: PoolUuid) -> String {
    uuid.to_string()
}

pub fn name_prop<'a>(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Str<'a> {
    let (name, _, _) = guard.as_tuple();
    Str::from(name.to_string())
}

pub fn size_prop<'a>(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Str<'a> {
    Str::from((*guard.total_physical_size()).to_string())
}

pub fn used_prop<'a>(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Structure<'a> {
    let (b, used) = option_to_tuple(
        guard.total_physical_used().map(|s| s.bytes()),
        Bytes::from(0u64),
    );
    Structure::from((b, (*used).to_string()))
}

pub fn allocated_prop<'a>(guard: SomeLockReadGuard<PoolUuid, dyn Pool>) -> Str<'a> {
    Str::from((*guard.total_allocated_size()).to_string())
}
