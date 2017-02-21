// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Macro for pool not found error on failure to get pool uuid.
macro_rules! get_pool_uuid_not_found_error {
    ( $object_path:ident; $context:ident; $default:ident; $message:ident ) => {
        if let Some(tuple) = $context.pools.borrow().get(&$object_path) {
            tuple.1.clone()
        } else {
            let message = format!("no pool for object path {}", $object_path);
            let (rc, rs) = code_to_message_items(DbusErrorEnum::POOL_NOTFOUND, message);
            return Ok(vec![$message.append3($default, rc, rs)]);
        }
    }
}

/// Macro for internal error on failure to get pool uuid.
macro_rules! get_pool_uuid_internal_error {
    ( $object_path:ident; $context:ident; $default:ident; $message:ident ) => {
        if let Some(tuple) = $context.pools.borrow().get(&$object_path) {
            tuple.1.clone()
        } else {
            let (rc, rs) = code_to_message_items(DbusErrorEnum::INTERNAL_ERROR,
                                                 format!("no entry for pool object path {}",
                                                         $object_path));
            return Ok(vec![$message.append3($default, rc, rs)]);
        }
    }
}

/// Macro for internal error on failure to get filesystem tuple.
macro_rules! get_fs_tuple_internal_error {
    ( $object_path:ident; $context:ident; $default:ident; $message:ident ) => {
        if let Some(tuple) = $context.filesystems.borrow().get(&$object_path) {
            tuple.clone()
        } else {
            let (rc, rs) = code_to_message_items(DbusErrorEnum::INTERNAL_ERROR,
                                                 format!("no entry for filesystem object path {}",
                                                         $object_path));
            return Ok(vec![$message.append3($default, rc, rs)]);
        }
    }
}


/// Macro for early return with Ok dbus message on failure to get pool.
macro_rules! get_pool {
    ( $engine:ident; $uuid:ident; $default:expr; $message:expr ) => {
        if let Some(pool) = $engine.get_pool(&$uuid) {
            pool
        } else {
            let message = format!("engine does not know about pool with uuid {}",
                                  $uuid);
            let (rc, rs) = code_to_message_items(DbusErrorEnum::INTERNAL_ERROR, message);
            return Ok(vec![$message.append3($default, rc, rs)]);
        }
    }
}
