// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

macro_rules! log_on_failure {
    ($op:expr, $fmt:tt $(, $arg:expr)*) => {{
        let result = $op;
        if let Err(ref e) = result {
            warn!(
                concat!($fmt, "; failed with error: {}"),
                $($arg,)*
                e
            );
        }
        result?
    }}
}
