#[cfg(feature = "dbus_enabled")]
macro_rules! mutex_lock {
    ($mutex:expr, $default_return:expr, $return_message:expr) => {
        match $mutex.lock() {
            Ok(lock) => lock,
            Err(e) => {
                return Ok(vec![$return_message.append3(
                    $default_return,
                    $crate::dbus_api::types::DbusErrorEnum::ERROR as u16,
                    e.to_string(),
                )]);
            }
        }
    };
    ($mutex:expr) => {
        $mutex.lock()?
    };
    ($mutex:expr, $map_err:expr) => {
        $mutex.lock().map_err($map_err)?
    };
}
