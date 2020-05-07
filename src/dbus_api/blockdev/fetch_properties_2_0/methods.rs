// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus::{
    arg::{RefArg, Variant},
    tree::{MTFn, MethodInfo, MethodResult},
    Message,
};
use itertools::Itertools;

use crate::dbus_api::{
    blockdev::shared::blockdev_operation, consts, types::TData, util::result_to_tuple,
};

const ALL_PROPERTIES: [&str; 1] = [consts::BLOCKDEV_TOTAL_SIZE_PROP];

fn get_properties_shared(
    m: &MethodInfo<MTFn<TData>, TData>,
    properties: &mut dyn Iterator<Item = String>,
) -> MethodResult {
    let message: &Message = m.msg;
    let object_path = &m.path;

    let return_message = message.method_return();

    let return_value: HashMap<String, (bool, Variant<Box<dyn RefArg>>)> =
        properties
            .unique()
            .filter_map(|prop| match prop.as_str() {
                consts::BLOCKDEV_TOTAL_SIZE_PROP => Some((
                    prop,
                    result_to_tuple(blockdev_operation(
                        m.tree,
                        object_path.get_name(),
                        |_, bd| {
                            Ok((u128::from(*bd.size()) * devicemapper::SECTOR_SIZE as u128)
                                .to_string())
                        },
                    )),
                )),
                _ => None,
            })
            .collect();

    Ok(vec![return_message.append1(return_value)])
}

properties_footer!();
