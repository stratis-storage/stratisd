// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod api;
mod org_storage_stratis1;

pub use self::api::StratisVarlinkService;
pub use self::org_storage_stratis1::{r#Pool, VarlinkClient, VarlinkClientInterface};
