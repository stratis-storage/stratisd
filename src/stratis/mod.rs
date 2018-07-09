// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub use self::errors::{ErrorEnum, StratisError, StratisResult};
pub use self::stratis::VERSION;

pub mod buff_log;
mod errors;
#[allow(module_inception)]
mod stratis;
