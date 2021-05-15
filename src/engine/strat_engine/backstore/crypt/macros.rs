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

macro_rules! check_key {
    ($condition:expr, $key:tt, $value:tt) => {
        if $condition {
            return Err($crate::stratis::StratisError::Error(format!(
                "Stratis token key '{}' requires a value of '{}'",
                $key, $value,
            )));
        }
    };
}

macro_rules! check_and_get_key {
    ($get:expr, $key:tt) => {
        if let Some(v) = $get {
            v
        } else {
            return Err($crate::stratis::StratisError::Error(format!(
                "Stratis token is missing key '{}' or the value is of the wrong type",
                $key
            )));
        }
    };
    ($get:expr, $func:expr, $key:tt, $ty:ty) => {
        if let Some(ref v) = $get {
            $func(v).map_err(|e| {
                $crate::stratis::StratisError::Error(format!(
                    "Failed to convert value for key '{}' to type {}: {}",
                    $key,
                    stringify!($ty),
                    e
                ))
            })?
        } else {
            return Err($crate::stratis::StratisError::Error(format!(
                "Stratis token is missing key '{}' or the value is of the wrong type",
                $key
            )));
        }
    };
}
