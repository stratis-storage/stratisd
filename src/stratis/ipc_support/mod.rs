// FIXME: cargo does not support mutually exclusive features. Because of this,
// our clippy Makefile target will enable both the dbus_enabled and min features.
// Currently, this module employs a workaround which will use the no_support module
// if the mutually exclusive features are enabled. This approach does not scale
// well if we add additional IPC mechanisms and a better solution should be
// considered.

#[cfg(all(feature = "dbus_enabled", not(feature = "min")))]
mod dbus_support;
#[cfg(all(feature = "min", not(feature = "dbus_enabled")))]
mod jsonrpc_support;
#[cfg(any(
    not(any(feature = "dbus_enabled", feature = "min")),
    all(feature = "dbus_enabled", feature = "min")
))]
mod no_support;

#[cfg(all(feature = "dbus_enabled", not(feature = "min")))]
pub use dbus_support::setup;

#[cfg(all(feature = "min", not(feature = "dbus_enabled")))]
pub use jsonrpc_support::setup;

#[cfg(any(
    not(any(feature = "dbus_enabled", feature = "min")),
    all(feature = "dbus_enabled", feature = "min")
))]
pub use no_support::setup;
