// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{fmt, io};
use std::error::Error;
use std::str;

#[cfg(feature="dbus_enabled")]
use dbus;
use libudev;
use nix;
use uuid;
use serde_json;

use devicemapper;

pub type StratisResult<T> = Result<T, StratisError>;

#[derive(Debug, Clone)]
pub enum ErrorEnum {
    Error,

    AlreadyExists,
    Busy,
    Invalid,
    NotFound,
}

#[derive(Debug)]
pub enum StratisError {
    /// Error encountered on startup, before engine can be initialized
    Startup(String),
    Engine(ErrorEnum, String),
    Io(io::Error),
    Nix(nix::Error),
    Uuid(uuid::ParseError),
    Utf8(str::Utf8Error),
    Serde(serde_json::error::Error),
    DM(devicemapper::DmError),

    #[cfg(feature="dbus_enabled")]
    Dbus(dbus::Error),
    Udev(libudev::Error),
}

impl fmt::Display for StratisError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            StratisError::Startup(ref s) => write!(f, "Startup error: {}", s),
            StratisError::Engine(_, ref msg) => write!(f, "Engine error: {}", msg),
            StratisError::Io(ref err) => write!(f, "IO error: {}", err),
            StratisError::Nix(ref err) => write!(f, "Nix error: {}", err),
            StratisError::Uuid(ref err) => write!(f, "Uuid error: {}", err),
            StratisError::Utf8(ref err) => write!(f, "Utf8 error: {}", err),
            StratisError::Serde(ref err) => write!(f, "Serde error: {}", err),
            StratisError::DM(ref err) => write!(f, "DM error: {}", err),

            #[cfg(feature="dbus_enabled")]
            StratisError::Dbus(ref err) => {
                write!(f, "Dbus error: {}", err.message().unwrap_or("Unknown"))
            }
            StratisError::Udev(ref err) => write!(f, "Udev error: {}", err),
        }
    }
}

impl Error for StratisError {
    fn description(&self) -> &str {
        match *self {
            StratisError::Startup(ref s) => s,
            StratisError::Engine(_, ref msg) => msg,
            StratisError::Io(ref err) => err.description(),
            StratisError::Nix(ref err) => err.description(),
            StratisError::Uuid(_) => "Uuid::ParseError",
            StratisError::Utf8(ref err) => err.description(),
            StratisError::Serde(ref err) => err.description(),
            StratisError::DM(ref err) => err.description(),

            #[cfg(feature="dbus_enabled")]
            StratisError::Dbus(ref err) => err.message().unwrap_or("D-Bus Error"),
            StratisError::Udev(ref err) => Error::description(err),
        }
    }

    fn cause(&self) -> Option<&Error> {
        match *self {
            StratisError::Startup(_) |
            StratisError::Engine(_, _) => None,
            StratisError::Io(ref err) => Some(err),
            StratisError::Nix(ref err) => Some(err),
            StratisError::Uuid(ref err) => Some(err),
            StratisError::Utf8(ref err) => Some(err),
            StratisError::Serde(ref err) => Some(err),
            StratisError::DM(ref err) => Some(err),

            #[cfg(feature="dbus_enabled")]
            StratisError::Dbus(ref err) => Some(err),
            StratisError::Udev(ref err) => Some(err),
        }
    }
}

impl From<io::Error> for StratisError {
    fn from(err: io::Error) -> StratisError {
        StratisError::Io(err)
    }
}

impl From<nix::Error> for StratisError {
    fn from(err: nix::Error) -> StratisError {
        StratisError::Nix(err)
    }
}

impl From<uuid::ParseError> for StratisError {
    fn from(err: uuid::ParseError) -> StratisError {
        StratisError::Uuid(err)
    }
}

impl From<str::Utf8Error> for StratisError {
    fn from(err: str::Utf8Error) -> StratisError {
        StratisError::Utf8(err)
    }
}

impl From<serde_json::error::Error> for StratisError {
    fn from(err: serde_json::error::Error) -> StratisError {
        StratisError::Serde(err)
    }
}

impl From<devicemapper::DmError> for StratisError {
    fn from(err: devicemapper::DmError) -> StratisError {
        StratisError::DM(err)
    }
}

#[cfg(feature="dbus_enabled")]
impl From<dbus::Error> for StratisError {
    fn from(err: dbus::Error) -> StratisError {
        StratisError::Dbus(err)
    }
}

impl From<libudev::Error> for StratisError {
    fn from(err: libudev::Error) -> StratisError {
        StratisError::Udev(err)
    }
}
