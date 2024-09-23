// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_tree::Factory;

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, InterfacesAddedThreadSafe, OPContext},
        util::make_object_path,
    },
    engine::{BlockDev, BlockDevTier, DevUuid, StratisUuid},
};

mod blockdev_3_0;
mod blockdev_3_3;
pub mod prop_conv;
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
            Some(OPContext::new(parent.clone(), StratisUuid::Dev(uuid))),
        )
        .introspectable()
        .add(
            f.interface(consts::BLOCKDEV_INTERFACE_NAME_3_0, ())
                .add_m(blockdev_3_0::set_userid_method(&f))
                .add_p(blockdev_3_0::devnode_property(&f))
                .add_p(blockdev_3_0::hardware_info_property(&f))
                .add_p(blockdev_3_0::initialization_time_property(&f))
                .add_p(blockdev_3_0::pool_property(&f))
                .add_p(blockdev_3_0::tier_property(&f))
                .add_p(blockdev_3_0::user_info_property(&f))
                .add_p(blockdev_3_0::uuid_property(&f))
                .add_p(blockdev_3_0::physical_path_property(&f))
                .add_p(blockdev_3_0::size_property(&f)),
        )
        .add(
            f.interface(consts::BLOCKDEV_INTERFACE_NAME_3_1, ())
                .add_m(blockdev_3_0::set_userid_method(&f))
                .add_p(blockdev_3_0::devnode_property(&f))
                .add_p(blockdev_3_0::hardware_info_property(&f))
                .add_p(blockdev_3_0::initialization_time_property(&f))
                .add_p(blockdev_3_0::pool_property(&f))
                .add_p(blockdev_3_0::tier_property(&f))
                .add_p(blockdev_3_0::user_info_property(&f))
                .add_p(blockdev_3_0::uuid_property(&f))
                .add_p(blockdev_3_0::physical_path_property(&f))
                .add_p(blockdev_3_0::size_property(&f)),
        )
        .add(
            f.interface(consts::BLOCKDEV_INTERFACE_NAME_3_2, ())
                .add_m(blockdev_3_0::set_userid_method(&f))
                .add_p(blockdev_3_0::devnode_property(&f))
                .add_p(blockdev_3_0::hardware_info_property(&f))
                .add_p(blockdev_3_0::initialization_time_property(&f))
                .add_p(blockdev_3_0::pool_property(&f))
                .add_p(blockdev_3_0::tier_property(&f))
                .add_p(blockdev_3_0::user_info_property(&f))
                .add_p(blockdev_3_0::uuid_property(&f))
                .add_p(blockdev_3_0::physical_path_property(&f))
                .add_p(blockdev_3_0::size_property(&f)),
        )
        .add(
            f.interface(consts::BLOCKDEV_INTERFACE_NAME_3_3, ())
                .add_p(blockdev_3_0::devnode_property(&f))
                .add_p(blockdev_3_0::hardware_info_property(&f))
                .add_p(blockdev_3_0::initialization_time_property(&f))
                .add_p(blockdev_3_0::pool_property(&f))
                .add_p(blockdev_3_0::tier_property(&f))
                .add_p(blockdev_3_3::user_info_property(&f))
                .add_p(blockdev_3_0::uuid_property(&f))
                .add_p(blockdev_3_0::physical_path_property(&f))
                .add_p(blockdev_3_0::size_property(&f))
                .add_p(blockdev_3_3::new_size_property(&f)),
        )
        .add(
            f.interface(consts::BLOCKDEV_INTERFACE_NAME_3_4, ())
                .add_p(blockdev_3_0::devnode_property(&f))
                .add_p(blockdev_3_0::hardware_info_property(&f))
                .add_p(blockdev_3_0::initialization_time_property(&f))
                .add_p(blockdev_3_0::pool_property(&f))
                .add_p(blockdev_3_0::tier_property(&f))
                .add_p(blockdev_3_3::user_info_property(&f))
                .add_p(blockdev_3_0::uuid_property(&f))
                .add_p(blockdev_3_0::physical_path_property(&f))
                .add_p(blockdev_3_0::size_property(&f))
                .add_p(blockdev_3_3::new_size_property(&f)),
        )
        .add(
            f.interface(consts::BLOCKDEV_INTERFACE_NAME_3_5, ())
                .add_p(blockdev_3_0::devnode_property(&f))
                .add_p(blockdev_3_0::hardware_info_property(&f))
                .add_p(blockdev_3_0::initialization_time_property(&f))
                .add_p(blockdev_3_0::pool_property(&f))
                .add_p(blockdev_3_0::tier_property(&f))
                .add_p(blockdev_3_3::user_info_property(&f))
                .add_p(blockdev_3_0::uuid_property(&f))
                .add_p(blockdev_3_0::physical_path_property(&f))
                .add_p(blockdev_3_0::size_property(&f))
                .add_p(blockdev_3_3::new_size_property(&f)),
        )
        .add(
            f.interface(consts::BLOCKDEV_INTERFACE_NAME_3_6, ())
                .add_p(blockdev_3_0::devnode_property(&f))
                .add_p(blockdev_3_0::hardware_info_property(&f))
                .add_p(blockdev_3_0::initialization_time_property(&f))
                .add_p(blockdev_3_0::pool_property(&f))
                .add_p(blockdev_3_0::tier_property(&f))
                .add_p(blockdev_3_3::user_info_property(&f))
                .add_p(blockdev_3_0::uuid_property(&f))
                .add_p(blockdev_3_0::physical_path_property(&f))
                .add_p(blockdev_3_0::size_property(&f))
                .add_p(blockdev_3_3::new_size_property(&f)),
        )
        .add(
            f.interface(consts::BLOCKDEV_INTERFACE_NAME_3_7, ())
                .add_p(blockdev_3_0::devnode_property(&f))
                .add_p(blockdev_3_0::hardware_info_property(&f))
                .add_p(blockdev_3_0::initialization_time_property(&f))
                .add_p(blockdev_3_0::pool_property(&f))
                .add_p(blockdev_3_0::tier_property(&f))
                .add_p(blockdev_3_3::user_info_property(&f))
                .add_p(blockdev_3_0::uuid_property(&f))
                .add_p(blockdev_3_0::physical_path_property(&f))
                .add_p(blockdev_3_0::size_property(&f))
                .add_p(blockdev_3_3::new_size_property(&f)),
        );

    let path = object_path.get_name().to_owned();
    let interfaces = get_blockdev_properties(parent, uuid, tier, blockdev);
    dbus_context.push_add(object_path, interfaces);
    path
}

/// Get the initial state of all properties associated with a blockdev object.
pub fn get_blockdev_properties(
    parent: dbus::Path<'static>,
    dev_uuid: DevUuid,
    tier: BlockDevTier,
    dev: &dyn BlockDev,
) -> InterfacesAddedThreadSafe {
    initial_properties! {
        consts::BLOCKDEV_INTERFACE_NAME_3_0 => {
            consts::BLOCKDEV_DEVNODE_PROP => shared::blockdev_devnode_prop(dev),
            consts::BLOCKDEV_HARDWARE_INFO_PROP => shared::blockdev_hardware_info_prop(dev),
            consts::BLOCKDEV_USER_INFO_PROP => shared::blockdev_user_info_prop(dev),
            consts::BLOCKDEV_INIT_TIME_PROP => shared::blockdev_init_time_prop(dev),
            consts::BLOCKDEV_POOL_PROP => parent.clone(),
            consts::BLOCKDEV_UUID_PROP => uuid_to_string!(dev_uuid),
            consts::BLOCKDEV_TIER_PROP => shared::blockdev_tier_prop(tier),
            consts::BLOCKDEV_PHYSICAL_PATH_PROP => shared::blockdev_physical_path_prop(dev),
            consts::BLOCKDEV_TOTAL_SIZE_PROP => shared::blockdev_size_prop(dev)
        },
        consts::BLOCKDEV_INTERFACE_NAME_3_1 => {
            consts::BLOCKDEV_DEVNODE_PROP => shared::blockdev_devnode_prop(dev),
            consts::BLOCKDEV_HARDWARE_INFO_PROP => shared::blockdev_hardware_info_prop(dev),
            consts::BLOCKDEV_USER_INFO_PROP => shared::blockdev_user_info_prop(dev),
            consts::BLOCKDEV_INIT_TIME_PROP => shared::blockdev_init_time_prop(dev),
            consts::BLOCKDEV_POOL_PROP => parent.clone(),
            consts::BLOCKDEV_UUID_PROP => uuid_to_string!(dev_uuid),
            consts::BLOCKDEV_TIER_PROP => shared::blockdev_tier_prop(tier),
            consts::BLOCKDEV_PHYSICAL_PATH_PROP => shared::blockdev_physical_path_prop(dev),
            consts::BLOCKDEV_TOTAL_SIZE_PROP => shared::blockdev_size_prop(dev)
        },
        consts::BLOCKDEV_INTERFACE_NAME_3_2 => {
            consts::BLOCKDEV_DEVNODE_PROP => shared::blockdev_devnode_prop(dev),
            consts::BLOCKDEV_HARDWARE_INFO_PROP => shared::blockdev_hardware_info_prop(dev),
            consts::BLOCKDEV_USER_INFO_PROP => shared::blockdev_user_info_prop(dev),
            consts::BLOCKDEV_INIT_TIME_PROP => shared::blockdev_init_time_prop(dev),
            consts::BLOCKDEV_POOL_PROP => parent.clone(),
            consts::BLOCKDEV_UUID_PROP => uuid_to_string!(dev_uuid),
            consts::BLOCKDEV_TIER_PROP => shared::blockdev_tier_prop(tier),
            consts::BLOCKDEV_PHYSICAL_PATH_PROP => shared::blockdev_physical_path_prop(dev),
            consts::BLOCKDEV_TOTAL_SIZE_PROP => shared::blockdev_size_prop(dev)
        },
        consts::BLOCKDEV_INTERFACE_NAME_3_3 => {
            consts::BLOCKDEV_DEVNODE_PROP => shared::blockdev_devnode_prop(dev),
            consts::BLOCKDEV_HARDWARE_INFO_PROP => shared::blockdev_hardware_info_prop(dev),
            consts::BLOCKDEV_USER_INFO_PROP => shared::blockdev_user_info_prop(dev),
            consts::BLOCKDEV_INIT_TIME_PROP => shared::blockdev_init_time_prop(dev),
            consts::BLOCKDEV_POOL_PROP => parent.clone(),
            consts::BLOCKDEV_UUID_PROP => uuid_to_string!(dev_uuid),
            consts::BLOCKDEV_TIER_PROP => shared::blockdev_tier_prop(tier),
            consts::BLOCKDEV_PHYSICAL_PATH_PROP => shared::blockdev_physical_path_prop(dev),
            consts::BLOCKDEV_TOTAL_SIZE_PROP => shared::blockdev_size_prop(dev),
            consts::BLOCKDEV_NEW_SIZE_PROP => shared::blockdev_new_size_prop(dev)
        },
        consts::BLOCKDEV_INTERFACE_NAME_3_4 => {
            consts::BLOCKDEV_DEVNODE_PROP => shared::blockdev_devnode_prop(dev),
            consts::BLOCKDEV_HARDWARE_INFO_PROP => shared::blockdev_hardware_info_prop(dev),
            consts::BLOCKDEV_USER_INFO_PROP => shared::blockdev_user_info_prop(dev),
            consts::BLOCKDEV_INIT_TIME_PROP => shared::blockdev_init_time_prop(dev),
            consts::BLOCKDEV_POOL_PROP => parent.clone(),
            consts::BLOCKDEV_UUID_PROP => uuid_to_string!(dev_uuid),
            consts::BLOCKDEV_TIER_PROP => shared::blockdev_tier_prop(tier),
            consts::BLOCKDEV_PHYSICAL_PATH_PROP => shared::blockdev_physical_path_prop(dev),
            consts::BLOCKDEV_TOTAL_SIZE_PROP => shared::blockdev_size_prop(dev),
            consts::BLOCKDEV_NEW_SIZE_PROP => shared::blockdev_new_size_prop(dev)
        },
        consts::BLOCKDEV_INTERFACE_NAME_3_5 => {
            consts::BLOCKDEV_DEVNODE_PROP => shared::blockdev_devnode_prop(dev),
            consts::BLOCKDEV_HARDWARE_INFO_PROP => shared::blockdev_hardware_info_prop(dev),
            consts::BLOCKDEV_USER_INFO_PROP => shared::blockdev_user_info_prop(dev),
            consts::BLOCKDEV_INIT_TIME_PROP => shared::blockdev_init_time_prop(dev),
            consts::BLOCKDEV_POOL_PROP => parent.clone(),
            consts::BLOCKDEV_UUID_PROP => uuid_to_string!(dev_uuid),
            consts::BLOCKDEV_TIER_PROP => shared::blockdev_tier_prop(tier),
            consts::BLOCKDEV_PHYSICAL_PATH_PROP => shared::blockdev_physical_path_prop(dev),
            consts::BLOCKDEV_TOTAL_SIZE_PROP => shared::blockdev_size_prop(dev),
            consts::BLOCKDEV_NEW_SIZE_PROP => shared::blockdev_new_size_prop(dev)
        },
        consts::BLOCKDEV_INTERFACE_NAME_3_6 => {
            consts::BLOCKDEV_DEVNODE_PROP => shared::blockdev_devnode_prop(dev),
            consts::BLOCKDEV_HARDWARE_INFO_PROP => shared::blockdev_hardware_info_prop(dev),
            consts::BLOCKDEV_USER_INFO_PROP => shared::blockdev_user_info_prop(dev),
            consts::BLOCKDEV_INIT_TIME_PROP => shared::blockdev_init_time_prop(dev),
            consts::BLOCKDEV_POOL_PROP => parent.clone(),
            consts::BLOCKDEV_UUID_PROP => uuid_to_string!(dev_uuid),
            consts::BLOCKDEV_TIER_PROP => shared::blockdev_tier_prop(tier),
            consts::BLOCKDEV_PHYSICAL_PATH_PROP => shared::blockdev_physical_path_prop(dev),
            consts::BLOCKDEV_TOTAL_SIZE_PROP => shared::blockdev_size_prop(dev),
            consts::BLOCKDEV_NEW_SIZE_PROP => shared::blockdev_new_size_prop(dev)
        },
        consts::BLOCKDEV_INTERFACE_NAME_3_7 => {
            consts::BLOCKDEV_DEVNODE_PROP => shared::blockdev_devnode_prop(dev),
            consts::BLOCKDEV_HARDWARE_INFO_PROP => shared::blockdev_hardware_info_prop(dev),
            consts::BLOCKDEV_USER_INFO_PROP => shared::blockdev_user_info_prop(dev),
            consts::BLOCKDEV_INIT_TIME_PROP => shared::blockdev_init_time_prop(dev),
            consts::BLOCKDEV_POOL_PROP => parent,
            consts::BLOCKDEV_UUID_PROP => uuid_to_string!(dev_uuid),
            consts::BLOCKDEV_TIER_PROP => shared::blockdev_tier_prop(tier),
            consts::BLOCKDEV_PHYSICAL_PATH_PROP => shared::blockdev_physical_path_prop(dev),
            consts::BLOCKDEV_TOTAL_SIZE_PROP => shared::blockdev_size_prop(dev),
            consts::BLOCKDEV_NEW_SIZE_PROP => shared::blockdev_new_size_prop(dev)
        }
    }
}
