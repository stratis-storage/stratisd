// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::os::unix::io::{AsRawFd, RawFd};

use nix::{
    errno::Errno,
    sys::{
        socket::{recvmsg, ControlMessageOwned, MsgFlags},
        uio::IoVec,
    },
    unistd::close,
};

use crate::{
    jsonrpc::consts::{OP_ERR, OP_OK, OP_OK_STR},
    stratis::StratisResult,
};

#[macro_export]
macro_rules! default_handler {
    ($respond:expr, $fn:path, $engine:expr, $default_value:expr $(, $args:expr)*) => {
        $respond.ok($crate::jsonrpc::server::utils::stratis_result_to_return(
            $fn(
                $engine,
                $($args),*
            ),
            $default_value,
        )).await
    }
}

pub fn stratis_result_to_return<T>(result: StratisResult<T>, default_value: T) -> (T, u16, String) {
    match result {
        Ok(r) => (r, OP_OK, OP_OK_STR.to_string()),
        Err(e) => (default_value, OP_ERR, e.to_string()),
    }
}

pub fn get_fd_from_sock(sock_fd: RawFd) -> Result<RawFd, nix::Error> {
    let mut cmsg_space = cmsg_space!([RawFd; 1]);
    let r_msg = recvmsg(
        sock_fd,
        &[IoVec::from_mut_slice(&mut [0, 0, 0, 0])],
        Some(&mut cmsg_space),
        MsgFlags::empty(),
    )?;
    let mut cmsgs: Vec<_> = r_msg.cmsgs().collect();
    if cmsgs.len() != 1 {
        cmsgs
            .into_iter()
            .filter_map(|msg| {
                if let ControlMessageOwned::ScmRights(vec) = msg {
                    Some(vec)
                } else {
                    None
                }
            })
            .for_each(|vec| {
                for fd in vec {
                    if let Err(e) = close(fd) {
                        warn!("Failed to close file descriptor {}: {}", fd, e);
                    }
                }
            });
        Err(nix::Error::from_errno(Errno::EINVAL))
    } else {
        let c_msg = cmsgs.pop().expect("Length is 1");
        match c_msg {
            ControlMessageOwned::ScmRights(mut vec) => {
                if vec.len() != 1 {
                    for fd in vec {
                        if let Err(e) = close(fd) {
                            warn!("Failed to close file descriptor {}: {}", fd, e);
                        }
                    }
                    Err(nix::Error::from_errno(Errno::EINVAL))
                } else {
                    Ok(vec.pop().expect("Length is 1"))
                }
            }
            _ => Err(nix::Error::from_errno(Errno::EINVAL)),
        }
    }
}

pub struct OwnedFd(RawFd);

impl OwnedFd {
    pub fn new(fd: RawFd) -> OwnedFd {
        OwnedFd(fd)
    }
}

impl AsRawFd for OwnedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl Drop for OwnedFd {
    fn drop(&mut self) {
        if let Err(e) = close(self.0) {
            warn!("Could not clean up file descriptor {}: {}", self.0, e);
        }
    }
}
