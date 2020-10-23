#[cfg(feature = "dbus_enabled")]
mod dbus_support;
#[cfg(not(feature = "dbus_enabled"))]
mod no_support;

#[cfg(feature = "dbus_enabled")]
pub use dbus_support::setup;

#[cfg(not(feature = "dbus_enabled"))]
pub use no_support::setup;
