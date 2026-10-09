// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(not(feature = "md_raid"))]
mod dm_raid;
#[cfg(feature = "md_raid")]
mod md_raid;

#[cfg(not(feature = "md_raid"))]
pub use dm_raid::{set_up_raid_array, tear_down_raid, wait_on_sync_completion};
#[cfg(feature = "md_raid")]
pub use md_raid::{set_up_raid_array, tear_down_raid, wait_on_sync_completion};
