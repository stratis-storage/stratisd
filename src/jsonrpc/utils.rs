// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::os::unix::io::{AsRawFd, RawFd};

use nix::{
    errno::Errno,
    sys::{
        socket::{recvmsg, sendmsg, ControlMessage, ControlMessageOwned, MsgFlags},
        uio::IoVec,
    },
    unistd::close,
};

use crate::{jsonrpc::consts::OP_ERR, stratis::StratisError};

pub fn stratis_error_to_return(e: StratisError) -> (u16, String) {
    (OP_ERR, e.to_string())
}

pub fn send_fd_to_sock(unix_fd: RawFd, fd: RawFd) -> Result<(), nix::Error> {
    sendmsg(
        unix_fd,
        &[IoVec::from_slice(&[0, 0, 0, 0])],
        &[ControlMessage::ScmRights(&[fd])],
        MsgFlags::empty(),
        None,
    )?;
    Ok(())
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

#[macro_export]
macro_rules! do_request {
    ($fn:path $(, $args:expr)*) => {
        match async_std::task::block_on(async {
            let transport = jsonrpsee::transport::http::HttpTransportClient::new($crate::jsonrpc::consts::RPC_CONNADDR);
            let mut client = jsonrpsee::raw::RawClient::new(transport);
            $fn(&mut client $(, $args)*).await
        }) {
            Ok(r) => r,
            Err(e) => return Err(
                $crate::stratis::StratisError::Error(format!("Transport error: {}", e))
            ),
        }
    }
}

#[macro_export]
macro_rules! left_align {
    ($string:expr, $max_length:expr) => {{
        let len = $string.len();
        $string + vec![" "; $max_length - len + 3].join("").as_str()
    }};
}

#[macro_export]
macro_rules! right_align {
    ($string:expr, $max_length:expr) => {
        vec![" "; $max_length - $string.len() + 3].join("") + $string.as_str()
    };
}

#[macro_export]
macro_rules! align {
    ($string:expr, $max_length:expr, $align:tt) => {
        if $align == ">" {
            $crate::right_align!($string, $max_length)
        } else {
            $crate::left_align!($string, $max_length)
        }
    };
}

#[macro_export]
macro_rules! print_table {
    ($($heading:expr, $values:expr, $align:tt);*) => {{
        let (lengths_same, lengths) = vec![$($values.len()),*]
            .into_iter()
            .fold((true, None), |(is_same, len_opt), len| {
                if len_opt.is_none() {
                    (true, Some(len))
                } else {
                    (is_same && len_opt == Some(len), len_opt)
                }
            });
        if !lengths_same {
            return Err($crate::stratis::StratisError::Error(
                "All values parameters must be the same length".to_string()
            ));
        }
        let mut output = vec![String::new(); lengths.unwrap_or(0) + 1];
        $(
            let max_length = $values
                .iter()
                .fold($heading.len(), |acc, val| {
                    if val.len() > acc {
                        val.len()
                    } else {
                        acc
                    }
                });
            if let Some(string) = output.get_mut(0) {
                string.push_str($crate::align!($heading.to_string(), max_length, $align).as_str());
            }
            for (index, row_seg) in $values.into_iter()
                .map(|s| $crate::align!(s, max_length, $align))
                .enumerate()
            {
                if let Some(string) = output.get_mut(index + 1) {
                    string.push_str(row_seg.as_str());
                }
            }
        )*
        for row in output.into_iter() {
            println!("{}", row);
        }
    }};
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
