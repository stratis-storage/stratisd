// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;

use crate::{
    dbus::{consts::OK_STRING, types::DbusErrorEnum, util::engine_to_dbus_err_tuple},
    engine::{Engine, ReportType},
    stratis::StratisError,
};

pub async fn get_report_method(engine: &Arc<dyn Engine>, name: &str) -> (String, u16, String) {
    let default_return = String::new();

    let report_type = match ReportType::try_from(name)
        .map_err(|_| StratisError::Msg(format!("Report name {name} not understood")))
    {
        Ok(rt) => rt,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return (default_return, rc, rs);
        }
    };

    match serde_json::to_string(&engine.get_report(report_type)) {
        Ok(result) => (result, DbusErrorEnum::OK as u16, OK_STRING.to_string()),
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e.into());
            (default_return, rc, rs)
        }
    }
}
