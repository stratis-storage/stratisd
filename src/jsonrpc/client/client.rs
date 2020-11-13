// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![allow(dead_code)]

use std::{
    io::Read,
    os::unix::{
        io::{AsRawFd, RawFd},
        net::UnixStream,
    },
    path::Path,
};

use nix::sys::{
    socket::{sendmsg, ControlMessage, MsgFlags},
    uio::IoVec,
};

use crate::{
    jsonrpc::interface::{StratisParams, StratisRet},
    stratis::StratisResult,
};

fn send_request(unix_fd: RawFd, vec: Vec<u8>, fd_opt: Option<RawFd>) -> StratisResult<()> {
    let fd_vec: Vec<_> = fd_opt.into_iter().collect();
    let scm = if fd_vec.is_empty() {
        vec![]
    } else {
        vec![ControlMessage::ScmRights(fd_vec.as_slice())]
    };
    sendmsg(
        unix_fd,
        &[IoVec::from_slice(vec.as_slice())],
        scm.as_slice(),
        MsgFlags::empty(),
        None,
    )?;
    Ok(())
}

pub struct StratisClient(UnixStream);

impl StratisClient {
    fn connect<P>(path: P) -> StratisResult<StratisClient>
    where
        P: AsRef<Path>,
    {
        Ok(StratisClient(UnixStream::connect(path)?))
    }

    fn request(&mut self, params: StratisParams) -> StratisResult<StratisRet> {
        send_request(
            self.0.as_raw_fd(),
            serde_json::to_vec(&params.type_)?,
            params.fd_opt,
        )?;
        let mut vec = vec![0; 65536];
        let bytes_read = self.0.read(vec.as_mut_slice())?;
        vec.truncate(bytes_read);
        Ok(serde_json::from_slice(vec.as_slice())?)
    }
}
