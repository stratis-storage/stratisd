// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Engine level errors

pub enum Error {
    Cmd(crate::engine::strat_engine::cmd::Error),
}

impl From<crate::engine::strat_engine::cmd::Error> for Error {
    fn from(err: crate::engine::strat_engine::cmd::Error) -> Error {
        Error::Cmd(err)
    }
}
