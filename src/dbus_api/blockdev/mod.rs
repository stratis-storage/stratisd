// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{self, tree::Factory};
use uuid::Uuid;

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, OPContext},
        util::make_object_path,
    },
    engine::{BlockDev, MaybeDbusPath},
};

mod blockdev_2_0;
mod fetch_properties_2_0;
mod shared;

pub fn create_dbus_blockdev<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    uuid: Uuid,
    blockdev: &mut dyn BlockDev,
) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(object_name, Some(OPContext::new(parent, uuid)))
        .introspectable()
        .add(
            f.interface(consts::BLOCKDEV_INTERFACE_NAME, ())
                .add_m(blockdev_2_0::set_userid_method(&f))
                .add_p(blockdev_2_0::devnode_property(&f))
                .add_p(blockdev_2_0::hardware_info_property(&f))
                .add_p(blockdev_2_0::initialization_time_property(&f))
                .add_p(blockdev_2_0::pool_property(&f))
                .add_p(blockdev_2_0::tier_property(&f))
                .add_p(blockdev_2_0::user_info_property(&f))
                .add_p(blockdev_2_0::uuid_property(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME, ())
                .add_m(fetch_properties_2_0::get_all_properties_method(&f))
                .add_m(fetch_properties_2_0::get_properties_method(&f)),
        );

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    blockdev.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}
