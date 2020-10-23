// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::IterAppend;
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::{dbus_api::types::TData, stratis::VERSION};

pub fn get_version(
    i: &mut IterAppend,
    _p: &PropInfo<MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    i.append(VERSION);
    Ok(())
}
