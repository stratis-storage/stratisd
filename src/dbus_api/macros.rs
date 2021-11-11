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

macro_rules! typed_uuid_string_err {
    ($uuid:expr; $type:ident) => {
        match $uuid {
            $crate::engine::StratisUuid::$type(uuid) => uuid,
            ref u => {
                return Err(format!(
                    "expected {} UUID but found UUID with type {:?}",
                    stringify!($type),
                    u,
                ))
            }
        }
    };
}

macro_rules! typed_uuid {
    ($uuid:expr; $type:ident; $default:expr; $message:expr) => {
        if let $crate::engine::StratisUuid::$type(uuid) = $uuid {
            uuid
        } else {
            let message = format!(
                "expected {} UUID but found UUID with type {:?}",
                stringify!($type),
                $uuid,
            );
            let (rc, rs) = (
                $crate::dbus_api::types::DbusErrorEnum::INTERNAL_ERROR as u16,
                message,
            );
            return Ok(vec![$message.append3($default, rc, rs)]);
        }
    };
}

/// Macro for early return with Ok dbus message on failure to get immutable pool.
macro_rules! get_pool {
    ($engine:expr; $uuid:ident; $default:expr; $message:expr) => {
        if let Some(pool) = $engine.get_pool($uuid) {
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

/// Macro for early return with Ok dbus message on failure to get mutable pool.
macro_rules! get_mut_pool {
    ($engine:expr; $uuid:ident; $default:expr; $message:expr) => {
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
            m: &dbus_tree::MethodInfo<
                dbus_tree::MTSync<$crate::dbus_api::types::TData>,
                $crate::dbus_api::types::TData,
            >,
        ) -> dbus_tree::MethodResult {
            get_properties_shared(m, &mut ALL_PROPERTIES.iter().map(|&s| s.to_string()))
        }

        pub fn get_properties(
            m: &dbus_tree::MethodInfo<
                dbus_tree::MTSync<$crate::dbus_api::types::TData>,
                $crate::dbus_api::types::TData,
            >,
        ) -> dbus_tree::MethodResult {
            let message: &dbus::Message = m.msg;
            let mut iter = message.iter_init();
            let mut properties: dbus::arg::Array<String, _> =
                $crate::dbus_api::util::get_next_arg(&mut iter, 0)?;
            get_properties_shared(m, &mut properties)
        }
    };
}

macro_rules! initial_properties {
    ($($iface:expr => { $($prop:expr => $val:expr),* }),*) => {{
        let mut interfaces = vec![
            $(
                ($iface, vec![
                    $(
                        ($prop, dbus::arg::Variant(
                            Box::new($val) as Box<dyn dbus::arg::RefArg + std::marker::Send + std::marker::Sync>
                        )),
                    )*
                ]
                .into_iter()
                .map(|(s, v): (&str, dbus::arg::Variant<Box<dyn dbus::arg::RefArg + std::marker::Send + std::marker::Sync>>)| {
                    (s.to_string(), v)
                })
                .collect()),
            )*
        ]
        .into_iter()
        .map(|(s, v)| (s.to_string(), v))
        .collect::<$crate::dbus_api::types::InterfacesAddedThreadSafe>();
        interfaces.extend(
            $crate::dbus_api::consts::fetch_properties_interfaces()
                .into_iter()
                .map(|s| (s, std::collections::HashMap::new()))
        );
        interfaces
    }};
}

macro_rules! log_action {
    ($action:expr) => {{
        let action = $action;
        if let Ok(ref a) = action {
            log::info!("{}", a);
        }
        action
    }};
}
