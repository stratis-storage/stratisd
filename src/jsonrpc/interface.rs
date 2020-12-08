// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// These allows are needed for the autogenerated code in rpc_api!{}

use std::{convert::TryFrom, error::Error, os::unix::io::RawFd, path::PathBuf};

use serde_json::Value;

use crate::engine::{FilesystemUuid, KeyDescription, PoolUuid, UnlockMethod};

pub type PoolListType = (Vec<String>, Vec<(u64, Option<u64>)>, Vec<(bool, bool)>);
// FIXME: 4th tuple argument (String) can be implemented as a new type struct wrapping
// chrono::DateTime<Utc> as long as it implements serde::Serialize and
// serde::Deserialize.
pub type FsListType = (
    Vec<String>,
    Vec<String>,
    Vec<Option<u64>>,
    Vec<String>,
    Vec<PathBuf>,
    Vec<FilesystemUuid>,
);

pub struct JsonWithFd {
    pub json: Value,
    pub fd_opt: Option<RawFd>,
}

#[derive(Serialize, Deserialize)]
pub enum StratisParamType {
    KeySet(KeyDescription),
    KeyUnset(KeyDescription),
    KeyList,
    PoolCreate(String, Vec<PathBuf>, Option<KeyDescription>),
    PoolRename(String, String),
    PoolAddData(String, Vec<PathBuf>),
    PoolInitCache(String, Vec<PathBuf>),
    PoolAddCache(String, Vec<PathBuf>),
    PoolDestroy(String),
    PoolUnlock(UnlockMethod, Option<PoolUuid>),
    PoolList,
    PoolIsEncrypted(PoolUuid),
    PoolIsLocked(PoolUuid),
    PoolIsBound(PoolUuid),
    FsCreate(String, String),
    FsDestroy(String, String),
    FsRename(String, String, String),
    FsList,
    Report,
    Udev(String),
}

pub struct StratisParams {
    pub type_: StratisParamType,
    pub fd_opt: Option<RawFd>,
}

impl TryFrom<JsonWithFd> for StratisParams {
    type Error = Box<dyn Error>;

    fn try_from(json: JsonWithFd) -> Result<StratisParams, Box<dyn Error>> {
        Ok(StratisParams {
            type_: serde_json::from_value(json.json)?,
            fd_opt: json.fd_opt,
        })
    }
}

#[derive(Serialize, Deserialize)]
pub enum StratisRet {
    KeySet((Option<bool>, u16, String)),
    KeyUnset((bool, u16, String)),
    KeyList((Vec<KeyDescription>, u16, String)),
    PoolCreate((bool, u16, String)),
    PoolRename((bool, u16, String)),
    PoolAddData((bool, u16, String)),
    PoolInitCache((bool, u16, String)),
    PoolAddCache((bool, u16, String)),
    PoolDestroy((bool, u16, String)),
    PoolUnlock((bool, u16, String)),
    PoolList(PoolListType),
    PoolIsEncrypted((bool, u16, String)),
    PoolIsLocked((bool, u16, String)),
    PoolIsBound((bool, u16, String)),
    FsCreate((bool, u16, String)),
    FsList(FsListType),
    FsDestroy((bool, u16, String)),
    FsRename((bool, u16, String)),
    Report(Value),
    Udev((Option<(String, String)>, u16, String)),
}
