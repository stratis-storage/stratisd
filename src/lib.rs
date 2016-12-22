

extern crate devicemapper;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate nix;
extern crate crc;
extern crate bidir_map;
extern crate byteorder;
extern crate uuid;
extern crate time;
extern crate dbus;
extern crate term;
extern crate rand;
extern crate serde;


#[macro_use]
extern crate custom_derive;
#[macro_use]
extern crate newtype_derive;
#[macro_use]
extern crate enum_derive;

#[cfg(test)]
extern crate quickcheck;

pub static mut debug: bool = false;

macro_rules! dbgp {
    ($($arg:tt)*) => (
        unsafe {
            if ::debug {
                println!($($arg)*)
            }
        })
}


pub mod types;
pub mod consts;
pub mod dbus_consts;
pub mod stratis;
pub mod dbus_api;
pub mod engine;
