// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, iter::FromIterator};

use dbus::{
    arg::{RefArg, Variant},
    tree::Factory,
};
use uuid::Uuid;

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, OPContext, ObjectPathType},
        util::make_object_path,
    },
    engine::{BlockDev, BlockDevTier, DevUuid, MaybeDbusPath},
};

mod blockdev_2_0;
mod fetch_properties_2_0;
mod shared;

pub fn create_dbus_blockdev<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    uuid: Uuid,
    tier: BlockDevTier,
    blockdev: &mut dyn BlockDev,
) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(
            object_name,
            Some(OPContext::new(
                parent.clone(),
                uuid,
                ObjectPathType::Blockdev,
            )),
        )
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
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_1, ())
                .add_m(fetch_properties_2_0::get_all_properties_method(&f))
                .add_m(fetch_properties_2_0::get_properties_method(&f)),
        );

    let path = object_path.get_name().to_owned();
    let interfaces = get_inital_properties(parent, uuid, tier, blockdev);
    dbus_context
        .actions
        .borrow_mut()
        .push_add(object_path, interfaces);
    blockdev.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}

/// Get the initial state of all properties associated with a blockdev object.
pub fn get_inital_properties(
    parent: dbus::Path<'static>,
    dev_uuid: DevUuid,
    tier: BlockDevTier,
    dev: &dyn BlockDev,
) -> HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>> {
    let dev_iface_map = HashMap::from_iter(
        vec![
            (
                consts::BLOCKDEV_DEVNODE_PROP,
                Variant(
                    Box::new(dev.devnode().user_path().display().to_string()) as Box<dyn RefArg>
                ),
            ),
            (
                consts::BLOCKDEV_HARDWARE_INFO_PROP,
                Variant(Box::new(
                    dev.hardware_info()
                        .map_or_else(|| (false, "".to_owned()), |val| (true, val.to_owned())),
                ) as Box<dyn RefArg>),
            ),
            (
                consts::BLOCKDEV_USER_INFO_PROP,
                Variant(Box::new(
                    dev.user_info()
                        .map_or_else(|| (false, "".to_owned()), |val| (true, val.to_owned())),
                ) as Box<dyn RefArg>),
            ),
            (
                consts::BLOCKDEV_INIT_TIME_PROP,
                Variant(Box::new(dev.initialization_time().timestamp() as u64) as Box<dyn RefArg>),
            ),
            (
                consts::BLOCKDEV_POOL_PROP,
                Variant(Box::new(parent) as Box<dyn RefArg>),
            ),
            (
                consts::BLOCKDEV_UUID_PROP,
                Variant(Box::new(dev_uuid.to_simple_ref().to_string()) as Box<dyn RefArg>),
            ),
            (
                consts::BLOCKDEV_TIER_PROP,
                Variant(Box::new(tier as u16) as Box<dyn RefArg>),
            ),
        ]
        .into_iter()
        .map(|(key, val)| (key.to_string(), val)),
    );

    HashMap::from_iter(
        vec![
            (consts::BLOCKDEV_INTERFACE_NAME, dev_iface_map),
            (consts::PROPERTY_FETCH_INTERFACE_NAME, HashMap::new()),
            (consts::PROPERTY_FETCH_INTERFACE_NAME_2_1, HashMap::new()),
        ]
        .into_iter()
        .map(|(key, val)| (key.to_string(), val)),
    )
}
