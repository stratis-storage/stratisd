// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Macro for early return with Ok dbus message on failure to get data
/// associated with object path.
macro_rules! get_data {
    ($path:ident; $default:expr; $message:expr) => {
        if let Some(ref data) = *$path.get_data() {
            data
        } else {
            let message = format!("no data for object path {}", $path.get_name());
            let (rc, rs) = (
                $crate::dbus_api::types::DbusErrorEnum::INTERNAL_ERROR as u16,
                message,
            );
            return Ok(vec![$message.append3($default, rc, rs)]);
        }
    };
}

/// Macro for early return with Ok dbus message on failure to get parent
/// object path from tree.
macro_rules! get_parent {
    ($m:ident; $data:ident; $default:expr; $message:expr) => {
        if let Some(parent) = $m.tree.get(&$data.parent) {
            parent
        } else {
            let message = format!("no path for object path {}", $data.parent);
            let (rc, rs) = (
                $crate::dbus_api::types::DbusErrorEnum::INTERNAL_ERROR as u16,
                message,
            );
            return Ok(vec![$message.append3($default, rc, rs)]);
        }
    };
}

/// Macro for early return with Ok dbus message on failure to get mutable pool.
macro_rules! get_mut_pool {
    ($engine:ident; $uuid:ident; $default:expr; $message:expr) => {
        if let Some(pool) = $engine.get_mut_pool($uuid) {
            pool
        } else {
            let message = format!("engine does not know about pool with uuid {}", $uuid);
            let (rc, rs) = (
                $crate::dbus_api::types::DbusErrorEnum::INTERNAL_ERROR as u16,
                message,
            );
            return Ok(vec![$message.append3($default, rc, rs)]);
        }
    };
}

// Macro for formatting a Uuid object for transport on the D-Bus as a string
macro_rules! uuid_to_string {
    ($uuid:expr) => {
        $uuid.to_simple_ref().to_string()
    };
}

macro_rules! properties_footer {
    () => {
        pub fn get_all_properties(
            m: &dbus::tree::MethodInfo<
                dbus::tree::MTFn<$crate::dbus_api::types::TData>,
                $crate::dbus_api::types::TData,
            >,
        ) -> dbus::tree::MethodResult {
            get_properties_shared(m, &mut ALL_PROPERTIES.iter().map(|&s| s.to_string()))
        }

        pub fn get_properties(
            m: &dbus::tree::MethodInfo<
                dbus::tree::MTFn<$crate::dbus_api::types::TData>,
                $crate::dbus_api::types::TData,
            >,
        ) -> dbus::tree::MethodResult {
            let message: &dbus::Message = m.msg;
            let mut iter = message.iter_init();
            let mut properties: dbus::arg::Array<String, _> =
                $crate::dbus_api::util::get_next_arg(&mut iter, 0)?;
            get_properties_shared(m, &mut properties)
        }
    };
}

macro_rules! pool_op_logging {
    ($pre_oper:tt $(, $pre_args:expr)*; $post_oper:tt $(, $post_args:expr)*; $engine_op:expr) => {{
        info!($pre_oper, $($pre_args),*);
        let result = $engine_op;
        match result {
            Ok(ref action) => info!($post_oper, $($post_args,)* action),
            Err(ref err) => {
                warn!("pool operation failed with error: {}", err);
            }
        }
        result
    }};
}
