// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{
    self,
    arg::IterAppend,
    tree::{MTFn, MethodErr, PropInfo},
};

use crate::{
    dbus_api::{pool::shared::pool_operation, types::TData},
    engine::{Name, Pool, PoolUuid},
};

/// Get a pool property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// Pool and obtains the property from the pool.
pub fn get_pool_property<F, R>(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn((Name, PoolUuid, &dyn Pool)) -> Result<R, String>,
    R: dbus::arg::Append,
{
    i.append(
        pool_operation(p.tree, p.path.get_name(), getter).map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

pub fn get_pool_name(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(name, _, _)| Ok(name.to_owned()))
}
