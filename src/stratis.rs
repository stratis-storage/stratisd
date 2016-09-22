// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus_api::DbusContext;
use blockdev::BlockDevs;

#[derive(Debug, Clone)]
pub struct Stratis<'a> {
    pub id: String,
    pub name: String,
    pub dbus_context: Option<DbusContext<'a>>,
    pub block_devs: BlockDevs,
}

#[derive(Debug, Clone)]
pub enum StratisRunningState {
    Good,
    Degraded(u8),
}

#[derive(Debug, Clone)]
pub enum StratisState {
    Initializing,
    Good(StratisRunningState),
    RaidFailed,
    ThinPoolFailed,
    ThinFailed,
}

impl<'a> Stratis<'a> {


}
