
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
