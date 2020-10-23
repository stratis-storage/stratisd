// NOTE: cargo does not support mutually exclusive features. Because of this,
// our clippy Makefile target will enable both the dbus_enabled and min features.
// Currently, this module defaults to using D-Bus if both features are enabled.
// Both modules are declared if both features are active for the purpose of
// clippy.

#[cfg(feature = "dbus_enabled")]
mod dbus_support;
#[cfg(feature = "min")]
#[cfg_attr(feature = "dbus_enabled", allow(dead_code))]
mod jsonrpc_support;

#[cfg(feature = "dbus_enabled")]
pub use dbus_support::setup;

#[cfg(all(feature = "min", not(feature = "dbus_enabled")))]
pub use jsonrpc_support::setup;
