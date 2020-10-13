// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus::tree::Factory;

use crate::{
    dbus_api::{
        types::{DbusContext, InterfacesAdded, OPContext, ObjectPathType},
        util::make_object_path,
    },
    engine::{DeviceSet, MaybeDbusPath, PoolUuid},
};

pub fn create_dbus_device_set<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    uuid: PoolUuid,
    device_set: &mut dyn DeviceSet,
) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(
            object_name,
            Some(OPContext::new(parent, uuid, ObjectPathType::Pool)),
        )
        .introspectable();

    let path = object_path.get_name().to_owned();
    let interfaces = get_initial_properties(uuid);

    dbus_context
        .actions
        .borrow_mut()
        .push_add(object_path, interfaces);
    device_set.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}

/// Get the initial state of all properties associated with a device set object.
pub fn get_initial_properties(_pool_uuid: PoolUuid) -> InterfacesAdded {
    HashMap::new()
}
