// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

macro_rules! handle_action {
    ($action:expr) => {{
        let action = $action;
        if let Ok(ref a) = action {
            log::info!("{}", a);
        }
        action
    }};
    ($action:expr, $connection:expr, $manager:expr, $pool_uuid:expr) => {{
        let action = $action;
        if let Ok(ref a) = action {
            log::info!("{}", a);
        } else if let Err(ref e) = action {
            if e.error_to_available_actions().is_some() {
                match futures::executor::block_on($manager.read()).pool_get_path(&$pool_uuid) {
                    Some(p) => {
                        futures::executor::block_on($crate::dbus::util::send_action_availability_signal(&$connection, p));
                    }
                    None => {
                        log::warn!("Could not find path associated with pool with UUID {}; could not send action availability change signal", $pool_uuid);
                    }
                }
            }
        }
        action
    }};
}
