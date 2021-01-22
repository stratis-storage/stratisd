#[cfg(feature = "dbus_enabled")]
macro_rules! mutex_lock {
    ($mutex:expr) => {
        futures::executor::block_on($mutex.lock())
    };
}
