// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::stratis::{StratisError, StratisResult};

pub fn udev(dm_name: String) -> StratisResult<Option<(String, String)>> {
    let (opt, rc, rs) = do_request!(Udev, dm_name);
    if rc != 0 {
        Err(StratisError::Error(rs))
    } else {
        Ok(opt)
    }
}
