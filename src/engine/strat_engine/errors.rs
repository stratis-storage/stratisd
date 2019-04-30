// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Engine level errors

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Cmd(crate::engine::strat_engine::cmd::Error),
}

impl From<crate::engine::strat_engine::cmd::Error> for Error {
    fn from(err: crate::engine::strat_engine::cmd::Error) -> Error {
        Error::Cmd(err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::Cmd(ref err) => write!(f, "{}", err),
        }
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        match *self {
            Error::Cmd(ref err) => err.description(),
        }
    }

    fn cause(&self) -> Option<&dyn std::error::Error> {
        match *self {
            Error::Cmd(ref err) => Some(err),
        }
    }
}
