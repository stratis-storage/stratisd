// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::convert::TryFrom;

use dbus::{
    tree::{MTFn, MethodInfo, MethodResult},
    Message,
};

use crate::{
    dbus_api::{
        types::TData,
        util::{engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok},
    },
    engine::ReportType,
    stratis::StratisError,
};

pub fn get_report(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let report_name: &str = get_next_arg(&mut iter, 0)?;

    let return_message = message.method_return();
    let default_return = (false, String::new());

    let report_type = match ReportType::try_from(report_name) {
        Ok(rt) => rt,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    let dbus_context = m.tree.get_data();
    let engine = dbus_context.engine.borrow();

    let msg = match serde_json::to_string(&engine.get_report(report_type)) {
        Ok(string) => return_message.append3((true, string), msg_code_ok(), msg_string_ok()),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&StratisError::Serde(e));
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}
