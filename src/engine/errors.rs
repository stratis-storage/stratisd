// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io;
use std::fmt;
use std::error;

use nix;

#[derive(Debug, Clone)]
pub enum ErrorEnum {
    Error,

    AlreadyExists,
    Busy,
    Invalid,
    NotFound,
}

#[derive(Debug)]
pub enum EngineError {
    Engine(ErrorEnum, String),
    Io(io::Error),
    Nix(nix::Error),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            EngineError::Engine(_, ref msg) => write!(f, "Stratis error: {}", msg),
            EngineError::Io(ref err) => write!(f, "IO error: {}", err),
            EngineError::Nix(ref err) => write!(f, "Nix error: {}", err.errno().desc()),
        }
    }
}

impl error::Error for EngineError {
    fn description(&self) -> &str {
        match *self {
            EngineError::Engine(_, ref msg) => msg,
            EngineError::Io(ref err) => err.description(),
            EngineError::Nix(ref err) => err.errno().desc(),
        }
    }
}

pub type EngineResult<T> = Result<T, EngineError>;

impl From<io::Error> for EngineError {
    fn from(err: io::Error) -> EngineError {
        EngineError::Io(err)
    }
}

impl From<nix::Error> for EngineError {
    fn from(err: nix::Error) -> EngineError {
        EngineError::Nix(err)
    }
}
