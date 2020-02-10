// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{error::Error, fmt, io, str};

#[cfg(feature = "dbus_enabled")]
use dbus;
use libudev;
use nix;
use serde_json;
use uuid;

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
    Error(String),
    Engine(ErrorEnum, String),
    Io(io::Error),
    Nix(nix::Error),
    Uuid(uuid::parser::ParseError),
    Utf8(str::Utf8Error),
    Serde(serde_json::error::Error),
    DM(devicemapper::DmError),

    #[cfg(feature = "dbus_enabled")]
    Dbus(dbus::Error),
    Udev(libudev::Error),
}

impl fmt::Display for StratisError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            StratisError::Error(ref s) => write!(f, "Error: {}", s),
            StratisError::Engine(_, ref msg) => write!(f, "Engine error: {}", msg),
            StratisError::Io(ref err) => write!(f, "IO error: {}", err),
            StratisError::Nix(ref err) => write!(f, "Nix error: {}", err),
            StratisError::Uuid(ref err) => write!(f, "Uuid error: {}", err),
            StratisError::Utf8(ref err) => write!(f, "Utf8 error: {}", err),
            StratisError::Serde(ref err) => write!(f, "Serde error: {}", err),
            StratisError::DM(ref err) => write!(f, "DM error: {}", err),

            #[cfg(feature = "dbus_enabled")]
            StratisError::Dbus(ref err) => {
                write!(f, "Dbus error: {}", err.message().unwrap_or("Unknown"))
            }
            StratisError::Udev(ref err) => write!(f, "Udev error: {}", err),
        }
    }
}

impl Error for StratisError {
    fn cause(&self) -> Option<&dyn Error> {
        match *self {
            StratisError::Error(_) | StratisError::Engine(_, _) => None,
            StratisError::Io(ref err) => Some(err),
            StratisError::Nix(ref err) => Some(err),
            StratisError::Uuid(ref err) => Some(err),
            StratisError::Utf8(ref err) => Some(err),
            StratisError::Serde(ref err) => Some(err),
            StratisError::DM(ref err) => Some(err),

            #[cfg(feature = "dbus_enabled")]
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

impl From<uuid::parser::ParseError> for StratisError {
    fn from(err: uuid::parser::ParseError) -> StratisError {
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

#[cfg(feature = "dbus_enabled")]
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
