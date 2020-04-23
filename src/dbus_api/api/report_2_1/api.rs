// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Factory, MTFn, Method};

use crate::dbus_api::{api::report_2_1::methods::get_report, types::TData};

pub fn get_report_method(f: &Factory<MTFn<TData>, TData>) -> Method<MTFn<TData>, TData> {
    f.method("GetReport", (), get_report)
        .in_arg(("name", "s"))
        // The report is a JSON object.
        //
        // b: boolean that is false if the default value is being returned in the case
        //    of an error
        // s: string representation of a JSON object containing the report
        //
        // Rust representation: (bool, serde_json::Value)
        .out_arg(("result", "(bs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}
