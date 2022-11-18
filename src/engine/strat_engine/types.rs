// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use crate::{
    engine::{strat_engine::metadata::BDA, types::DevUuid},
    stratis::StratisError,
};

pub type BDAResult<T> = Result<T, (StratisError, BDA)>;
pub type BDARecordResult<T> = Result<T, (StratisError, HashMap<DevUuid, BDA>)>;
