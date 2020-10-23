#[cfg(feature = "dbus_enabled")]
macro_rules! mutex_lock {
    ($mutex:expr) => {
        async_std::task::block_on($mutex.lock())
    };
}
