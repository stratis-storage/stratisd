// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::error::Error;
use std::fmt;
use std::io;

#[cfg(feature="dbus_enabled")]
use dbus;

use term;

use engine::EngineError;

pub type StratisResult<T> = Result<T, StratisError>;

#[derive(Debug)]
pub enum StratisError {
    Engine(EngineError),
    StderrNotFound,
    Io(io::Error),

    #[cfg(feature="dbus_enabled")]
    Dbus(dbus::Error),

    Term(term::Error),
}

impl fmt::Display for StratisError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            StratisError::Engine(ref err) => {
                write!(f, "Engine error: {}", err.description().to_owned())
            }
            StratisError::StderrNotFound => write!(f, "stderr not found"),
            StratisError::Io(ref err) => write!(f, "IO error: {}", err),

            #[cfg(feature="dbus_enabled")]
            StratisError::Dbus(ref err) => {
                write!(f, "Dbus error: {}", err.message().unwrap_or("Unknown"))
            }

            StratisError::Term(ref err) => write!(f, "Term error: {}", err),
        }
    }
}

impl Error for StratisError {
    fn description(&self) -> &str {
        match *self {
            StratisError::Engine(ref err) => Error::description(err),
            StratisError::StderrNotFound => "stderr not found",
            StratisError::Io(ref err) => err.description(),

            #[cfg(feature="dbus_enabled")]
            StratisError::Dbus(ref err) => err.message().unwrap_or("D-Bus Error"),

            StratisError::Term(ref err) => Error::description(err),
        }
    }

    fn cause(&self) -> Option<&Error> {
        match *self {
            StratisError::Engine(ref err) => Some(err),
            StratisError::StderrNotFound => None,
            StratisError::Io(ref err) => Some(err),

            #[cfg(feature="dbus_enabled")]
            StratisError::Dbus(ref err) => Some(err),

            StratisError::Term(ref err) => Some(err),
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

impl From<term::Error> for StratisError {
    fn from(err: term::Error) -> StratisError {
        StratisError::Term(err)
    }
}

impl From<EngineError> for StratisError {
    fn from(err: EngineError) -> StratisError {
        StratisError::Engine(err)
    }
}
