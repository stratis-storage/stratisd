#![allow(dead_code)]
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::redundant_static_lifetimes)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::missing_safety_doc)]
// This allow should be removed once bindgen finds a way to
// generate struct alignment tests without triggering errors
// in the compiler. See https://github.com/rust-lang/rust-bindgen/issues/1651.
#![allow(unknown_lints)]
#![allow(deref_nullptr)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
