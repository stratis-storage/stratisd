// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{arg::Variant, tree::Factory};

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, InterfacesAdded, OPContext, ObjectPathType},
        util::make_object_path,
    },
    engine::{BlockDev, BlockDevTier, DevUuid, StratisUuid},
};

mod blockdev_2_0;
mod blockdev_2_2;
mod fetch_properties_2_0;
mod shared;

pub fn create_dbus_blockdev<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    uuid: DevUuid,
    tier: BlockDevTier,
    blockdev: &dyn BlockDev,
) -> dbus::Path<'a> {
    let f = Factory::new_sync();

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(
            object_name,
            Some(OPContext::new(
                parent.clone(),
                StratisUuid::Dev(uuid),
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
            f.interface(consts::BLOCKDEV_INTERFACE_NAME_2_2, ())
                .add_m(blockdev_2_0::set_userid_method(&f))
                .add_p(blockdev_2_0::devnode_property(&f))
                .add_p(blockdev_2_0::hardware_info_property(&f))
                .add_p(blockdev_2_0::initialization_time_property(&f))
                .add_p(blockdev_2_0::pool_property(&f))
                .add_p(blockdev_2_0::tier_property(&f))
                .add_p(blockdev_2_0::user_info_property(&f))
                .add_p(blockdev_2_0::uuid_property(&f))
                .add_p(blockdev_2_2::physical_path_property(&f)),
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
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_2, ())
                .add_m(fetch_properties_2_0::get_all_properties_method(&f))
                .add_m(fetch_properties_2_0::get_properties_method(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME_2_3, ())
                .add_m(fetch_properties_2_0::get_all_properties_method(&f))
                .add_m(fetch_properties_2_0::get_properties_method(&f)),
        );

    let path = object_path.get_name().to_owned();
    let interfaces = get_initial_properties(parent, uuid, tier, blockdev);
    dbus_context.push_add(object_path, interfaces);
    path
}

/// Get the initial state of all properties associated with a blockdev object.
pub fn get_initial_properties(
    parent: dbus::Path<'static>,
    dev_uuid: DevUuid,
    tier: BlockDevTier,
    dev: &dyn BlockDev,
) -> InterfacesAdded {
    initial_properties! {
        consts::BLOCKDEV_INTERFACE_NAME => {
            consts::BLOCKDEV_DEVNODE_PROP => shared::blockdev_devnode_prop(dev),
            consts::BLOCKDEV_HARDWARE_INFO_PROP => shared::blockdev_hardware_info_prop(dev),
            consts::BLOCKDEV_USER_INFO_PROP => shared::blockdev_user_info_prop(dev),
            consts::BLOCKDEV_INIT_TIME_PROP => shared::blockdev_init_time_prop(dev),
            consts::BLOCKDEV_POOL_PROP => parent.to_owned(),
            consts::BLOCKDEV_UUID_PROP => uuid_to_string!(dev_uuid),
            consts::BLOCKDEV_TIER_PROP => shared::blockdev_tier_prop(tier)
        },
        consts::BLOCKDEV_INTERFACE_NAME_2_2 => {
            consts::BLOCKDEV_DEVNODE_PROP => shared::blockdev_devnode_prop(dev),
            consts::BLOCKDEV_HARDWARE_INFO_PROP => shared::blockdev_hardware_info_prop(dev),
            consts::BLOCKDEV_USER_INFO_PROP => shared::blockdev_user_info_prop(dev),
            consts::BLOCKDEV_INIT_TIME_PROP => shared::blockdev_init_time_prop(dev),
            consts::BLOCKDEV_POOL_PROP => parent,
            consts::BLOCKDEV_UUID_PROP => uuid_to_string!(dev_uuid),
            consts::BLOCKDEV_TIER_PROP => shared::blockdev_tier_prop(tier),
            consts::BLOCKDEV_PHYSICAL_PATH_PROP => shared::blockdev_physical_path_prop(dev)
        }
    }
}
