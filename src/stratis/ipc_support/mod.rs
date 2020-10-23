#[cfg(feature = "dbus_enabled")]
mod dbus_support;

#[cfg(feature = "dbus_enabled")]
pub use dbus_support::IpcSupport;
