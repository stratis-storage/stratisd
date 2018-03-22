// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{fmt, io};
use std::error::Error;

#[cfg(feature="dbus_enabled")]
use dbus;
use libudev;

use engine::EngineError;

pub type StratisResult<T> = Result<T, StratisError>;

#[derive(Debug)]
pub enum StratisError {
    Error(String),
    Engine(EngineError),
    Io(io::Error),

    #[cfg(feature="dbus_enabled")]
    Dbus(dbus::Error),
    Udev(libudev::Error),
}

impl fmt::Display for StratisError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            StratisError::Error(ref s) => write!(f, "Error: {}", s),
            StratisError::Engine(ref err) => {
                write!(f, "Engine error: {}", err.description().to_owned())
            }
            StratisError::Io(ref err) => write!(f, "IO error: {}", err),

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
            StratisError::Error(ref s) => s,
            StratisError::Engine(ref err) => Error::description(err),
            StratisError::Io(ref err) => err.description(),

            #[cfg(feature="dbus_enabled")]
            StratisError::Dbus(ref err) => err.message().unwrap_or("D-Bus Error"),
            StratisError::Udev(ref err) => Error::description(err),
        }
    }

    fn cause(&self) -> Option<&Error> {
        match *self {
            StratisError::Error(_) => None,
            StratisError::Engine(ref err) => Some(err),
            StratisError::Io(ref err) => Some(err),
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

#[cfg(feature="dbus_enabled")]
impl From<dbus::Error> for StratisError {
    fn from(err: dbus::Error) -> StratisError {
        StratisError::Dbus(err)
    }
}

impl From<EngineError> for StratisError {
    fn from(err: EngineError) -> StratisError {
        StratisError::Engine(err)
    }
}

impl From<libudev::Error> for StratisError {
    fn from(err: libudev::Error) -> StratisError {
        StratisError::Udev(err)
    }
}
