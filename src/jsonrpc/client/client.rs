// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    io::{IoSlice, Read},
    os::unix::{
        io::{AsRawFd, RawFd},
        net::UnixStream,
    },
    path::Path,
};

use nix::sys::socket::{sendmsg, ControlMessage, MsgFlags, UnixAddr};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    jsonrpc::interface::{IpcResult, StratisParams, StratisRet},
    stratis::StratisResult,
};

fn send_request<S, D>(
    unix_stream: &mut UnixStream,
    msg: &S,
    fd_opt: Option<RawFd>,
) -> StratisResult<D>
where
    S: Serialize,
    D: DeserializeOwned,
{
    let vec = serde_json::to_vec(msg)?;
    let fd_vec: Vec<_> = fd_opt.into_iter().collect();
    let scm = if fd_vec.is_empty() {
        vec![]
    } else {
        vec![ControlMessage::ScmRights(fd_vec.as_slice())]
    };
    sendmsg::<UnixAddr>(
        unix_stream.as_raw_fd(),
        &[IoSlice::new(vec.as_slice())],
        scm.as_slice(),
        MsgFlags::empty(),
        None,
    )?;
    let mut vec = vec![0; 65536];
    let bytes_read = unix_stream.read(vec.as_mut_slice())?;
    vec.truncate(bytes_read);
    Ok(serde_json::from_slice(vec.as_slice())?)
}

pub struct StratisClient(UnixStream);

impl StratisClient {
    pub fn connect<P>(path: P) -> StratisResult<StratisClient>
    where
        P: AsRef<Path>,
    {
        Ok(StratisClient(UnixStream::connect(path)?))
    }

    pub fn request(&mut self, params: StratisParams) -> StratisResult<IpcResult<StratisRet>> {
        send_request(&mut self.0, &params.type_, params.fd_opt)
    }
}
