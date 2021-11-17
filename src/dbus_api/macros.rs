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
                $crate::dbus_api::types::DbusErrorEnum::ERROR as u16,
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
                $crate::dbus_api::types::DbusErrorEnum::ERROR as u16,
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
                $crate::dbus_api::types::DbusErrorEnum::ERROR as u16,
                message,
            );
            return Ok(vec![$message.append3($default, rc, rs)]);
        }
    };
}

/// Macro for early return with Ok dbus message on failure to get mutable pool.
macro_rules! get_mut_pool {
    ($engine:expr; $uuid:ident; $default:expr; $message:expr) => {
        if let Some(pool) =
            futures::executor::block_on($engine.get_mut_pool($crate::engine::LockKey::Uuid($uuid)))
        {
            pool
        } else {
            let message = format!("engine does not know about pool with uuid {}", $uuid);
            let (rc, rs) = (
                $crate::dbus_api::types::DbusErrorEnum::ERROR as u16,
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

macro_rules! initial_properties {
    ($($iface:expr => { $($prop:expr => $val:expr),* }),*) => {{
        vec![
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
        .collect::<$crate::dbus_api::types::InterfacesAddedThreadSafe>()
    }};
}

macro_rules! handle_action {
    ($action:expr) => {{
        let action = $action;
        if let Ok(ref a) = action {
            log::info!("{}", a);
        }
        action
    }};
    ($action:expr, $dbus_cxt:expr, $path:expr) => {{
        let action = $action;
        if let Ok(ref a) = action {
            log::info!("{}", a);
        } else if let Err(ref e) = action {
            if let Some(state) = e.error_to_available_actions() {
                $dbus_cxt.push_pool_avail_actions($path, state)
            }
        }
        action
    }};
}

macro_rules! box_variant {
    ($val:expr) => {
        dbus::arg::Variant(Box::new($val) as Box<dyn dbus::arg::RefArg>)
    };
}

macro_rules! pool_notify_lock {
    ($expr:expr, $return_message:expr, $default_return:expr) => {{
        match $expr {
            Ok(g) => g,
            Err(e) => {
                let (rc, rs) = $crate::dbus_api::util::engine_to_dbus_err_tuple(
                    &$crate::stratis::StratisError::Msg(format!(
                        "stratisd has ended up in a bad state and is no longer able to report pool creation completion: {}",
                        e,
                    ))
                );
                return Ok(vec![$return_message.append3($default_return, rc, rs)]);
            },
        }
    }}
}
