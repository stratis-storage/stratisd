// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Factory, MTSync, Method};

use crate::dbus_api::{api::manager_2_4::methods::engine_state_report, types::TData};

pub fn engine_state_report_method(
    f: &Factory<MTSync<TData>, TData>,
) -> Method<MTSync<TData>, TData> {
    f.method("EngineStateReport", (), engine_state_report)
        // s: JSON engine state report as a string.
        //
        // Rust representation: Value
        .out_arg(("result", "s"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"))
}
