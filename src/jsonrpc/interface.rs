// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// These allows are needed for the autogenerated code in rpc_api!{}

use std::{os::unix::io::RawFd, path::PathBuf};

use serde_json::Value;

use crate::engine::{
    FilesystemUuid, InputEncryptionInfo, KeyDescription, OptionalTokenSlotInput, PoolIdentifier,
    PoolUuid, TokenUnlockMethod,
};

pub type PoolListType = (
    Vec<String>,
    Vec<(u128, Option<u128>)>,
    Vec<(bool, bool)>,
    Vec<PoolUuid>,
);
// FIXME: 4th tuple argument (String) can be implemented as a new type struct wrapping
// chrono::DateTime<Utc> as long as it implements serde::Serialize and
// serde::Deserialize.
pub type FsListType = (
    Vec<String>,
    Vec<String>,
    Vec<Option<u128>>,
    Vec<String>,
    Vec<PathBuf>,
    Vec<FilesystemUuid>,
);

#[derive(Serialize, Deserialize)]
pub enum StratisParamType {
    KeySet(KeyDescription),
    KeyUnset(KeyDescription),
    KeyList,
    PoolCreate(String, Vec<PathBuf>, Option<InputEncryptionInfo>),
    PoolRename(String, String),
    PoolAddData(String, Vec<PathBuf>),
    PoolInitCache(String, Vec<PathBuf>),
    PoolAddCache(String, Vec<PathBuf>),
    PoolDestroy(String),
    PoolStart(PoolIdentifier<PoolUuid>, TokenUnlockMethod),
    PoolStop(PoolIdentifier<PoolUuid>),
    PoolList,
    PoolBindKeyring(
        PoolIdentifier<PoolUuid>,
        OptionalTokenSlotInput,
        KeyDescription,
    ),
    PoolBindClevis(
        PoolIdentifier<PoolUuid>,
        OptionalTokenSlotInput,
        String,
        Value,
    ),
    PoolUnbindKeyring(PoolIdentifier<PoolUuid>, Option<u32>),
    PoolUnbindClevis(PoolIdentifier<PoolUuid>, Option<u32>),
    PoolRebindKeyring(PoolIdentifier<PoolUuid>, Option<u32>, KeyDescription),
    PoolRebindClevis(PoolIdentifier<PoolUuid>, Option<u32>),
    PoolIsEncrypted(PoolIdentifier<PoolUuid>),
    PoolIsStopped(PoolIdentifier<PoolUuid>),
    PoolHasPassphrase(PoolIdentifier<PoolUuid>),
    PoolIsBound(PoolIdentifier<PoolUuid>),
    FsCreate(String, String),
    FsDestroy(String, String),
    FsRename(String, String, String),
    FsOrigin(String, String),
    FsList,
    Report,
}

pub struct StratisParams {
    pub type_: StratisParamType,
    pub fd_opt: Option<RawFd>,
}

pub type IpcResult<T> = Result<T, String>;

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
    PoolStart((bool, u16, String)),
    PoolStop((bool, u16, String)),
    PoolList(PoolListType),
    PoolBindKeyring((bool, u16, String)),
    PoolBindClevis((bool, u16, String)),
    PoolUnbindKeyring((bool, u16, String)),
    PoolUnbindClevis((bool, u16, String)),
    PoolRebindKeyring((bool, u16, String)),
    PoolRebindClevis((bool, u16, String)),
    PoolIsEncrypted((bool, u16, String)),
    PoolIsStopped((bool, u16, String)),
    PoolHasPassphrase((bool, u16, String)),
    PoolIsBound((bool, u16, String)),
    FsCreate((bool, u16, String)),
    FsList(FsListType),
    FsDestroy((bool, u16, String)),
    FsRename((bool, u16, String)),
    FsOrigin((Option<String>, u16, String)),
    Report(Value),
}
