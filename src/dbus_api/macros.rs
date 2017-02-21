// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.


/// Macro for early return with Ok dbus message on failure to get pool.
macro_rules! get_pool {
    ( $engine:ident; $uuid:ident; $default:expr; $message:expr ) => {
        if let Some(pool) = $engine.get_pool(&$uuid) {
            pool
        } else {
            let (rc, rs) = code_to_message_items(DbusErrorEnum::POOL_NOTFOUND,
                                                 format!("no pool for uuid {}", $uuid));
            return Ok(vec![$message.append3($default, rc, rs)]);
        }
    }
}

/// Macros for early return with an Ok dbus message on a dbus error
macro_rules! dbus_try {
    ( $val:expr; $default:expr; $message:expr ) => {
        match $val {
            Ok(v) => v,
            Err((rc, rs)) => {
                return Ok(vec![$message.append3($default, rc, rs)]);
            }
        };
    }
}
