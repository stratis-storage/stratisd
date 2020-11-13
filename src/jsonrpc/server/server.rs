// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![allow(dead_code)]

use std::{
    future::Future,
    io::{self, ErrorKind},
    os::unix::io::{AsRawFd, RawFd},
    path::Path,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::ready;
use nix::{
    errno::Errno,
    sys::{
        socket::{
            accept, bind, listen, recvmsg, socket, AddressFamily, ControlMessageOwned, MsgFlags,
            SockAddr, SockFlag, SockType,
        },
        uio::IoVec,
    },
    unistd::close,
};
use tokio::{
    io::unix::AsyncFd,
    stream::{Stream, StreamExt},
    sync::Mutex,
};

use crate::{
    engine::Engine,
    jsonrpc::{
        consts::OP_ERR,
        interface::{StratisParamType, StratisParams, StratisRet},
        server::{key, utils::stratis_result_to_return},
    },
    stratis::{StratisError, StratisResult},
};

impl StratisParams {
    async fn process(self, engine: Arc<Mutex<dyn Engine>>) -> StratisRet {
        match self.type_ {
            StratisParamType::KeySet(key_desc) => {
                let fd = match self.fd_opt {
                    Some(fd) => fd,
                    None => {
                        return StratisRet::KeySet(
                            None,
                            OP_ERR,
                            "No file descriptor provided to KeySet".to_string(),
                        )
                    }
                };
                let (bool_opt, rc, rs) =
                    stratis_result_to_return(key::key_set(engine, &key_desc, fd).await, None);
                StratisRet::KeySet(bool_opt, rc, rs)
            }
            StratisParamType::KeyUnset(key_desc) => {
                let (bool_val, rc, rs) =
                    stratis_result_to_return(key::key_unset(engine, &key_desc).await, false);
                StratisRet::KeyUnset(bool_val, rc, rs)
            }
        }
    }
}

pub struct StratisServer {
    engine: Arc<Mutex<dyn Engine>>,
    listener: StratisUnixListener,
}

impl StratisServer {
    pub async fn handle_request(&mut self) -> StratisResult<Option<()>> {
        let request = match self.listener.next().await {
            Some(req_res) => req_res?,
            None => return Ok(None),
        };
        tokio::spawn(async move {
            let _params = request.await;
        });
        Ok(Some(()))
    }
}

pub struct FdRef(RawFd);

impl FdRef {
    pub fn new(fd: RawFd) -> FdRef {
        FdRef(fd)
    }
}

impl AsRawFd for FdRef {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

fn handle_cmsgs(mut cmsgs: Vec<ControlMessageOwned>) -> StratisResult<Option<RawFd>> {
    if cmsgs.len() > 1 {
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
        return Err(StratisError::Error(
            "Unix packet contained more than one ancillary data message".to_string(),
        ));
    }

    Ok(match cmsgs.pop() {
        Some(ControlMessageOwned::ScmRights(mut vec)) => {
            if vec.len() > 1 {
                for fd in vec {
                    if let Err(e) = close(fd) {
                        warn!("Failed to close file descriptor {}: {}", fd, e);
                    }
                }
                return Err(StratisError::Error(
                    "Received more than one file descriptor".to_string(),
                ));
            } else {
                vec.pop()
            }
        }
        _ => None,
    })
}

fn poll_recvmsg(fd: RawFd) -> Poll<StratisResult<StratisParams>> {
    let mut cmsg_space = cmsg_space!([RawFd; 1]);
    let mut vec = vec![0; 65536];
    let rmsg_result = recvmsg(
        fd,
        &[IoVec::from_mut_slice(vec.as_mut_slice())],
        Some(&mut cmsg_space),
        MsgFlags::empty(),
    );

    let recvmsg_res = match rmsg_result {
        Ok(r) => Ok((r, r.cmsgs().collect())),
        Err(e) => {
            if let Some(errno) = e.as_errno() {
                if errno == Errno::EAGAIN {
                    return Poll::Pending;
                }
            }
            Err(StratisError::from(e))
        }
    };
    Poll::Ready(
        recvmsg_res
            .and_then(|(r, c)| handle_cmsgs(c).map(|fd_opt| (r, fd_opt)))
            .and_then(|(r, fd_opt)| {
                vec.truncate(r.bytes);
                serde_json::from_slice(vec.as_slice())
                    .map(|type_: StratisParamType| StratisParams { type_, fd_opt })
                    .map_err(StratisError::from)
            }),
    )
}

pub struct StratisUnixRequest(AsyncFd<FdRef>);

impl Future for StratisUnixRequest {
    type Output = StratisResult<StratisParams>;

    fn poll(self: Pin<&mut Self>, ctxt: &mut Context) -> Poll<StratisResult<StratisParams>> {
        let poll_res = ready!(self.0.poll_read_ready(ctxt));
        let mut poll_guard = match poll_res {
            Ok(poll) => poll,
            Err(e) => return Poll::Ready(Err(StratisError::from(e))),
        };
        poll_guard.with_poll(|| poll_recvmsg(self.0.as_raw_fd()))
    }
}

pub struct StratisUnixListener(AsyncFd<FdRef>);

impl StratisUnixListener {
    pub fn bind<P>(path: P) -> StratisResult<StratisUnixListener>
    where
        P: AsRef<Path>,
    {
        let fd = socket(
            AddressFamily::Unix,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )?;
        bind(fd, &SockAddr::new_unix(path.as_ref())?)?;
        listen(fd, 0)?;
        Ok(StratisUnixListener(AsyncFd::new(FdRef::new(fd))?))
    }
}

impl Stream for StratisUnixListener {
    type Item = StratisResult<StratisUnixRequest>;

    fn poll_next(
        self: Pin<&mut Self>,
        ctxt: &mut Context,
    ) -> Poll<Option<StratisResult<StratisUnixRequest>>> {
        let poll_res = ready!(self.0.poll_read_ready(ctxt));
        let mut poll_guard = match poll_res {
            Ok(poll) => poll,
            Err(e) => return Poll::Ready(Some(Err(StratisError::from(e)))),
        };

        Poll::Ready(Some(
            poll_guard
                .with_io(|| match accept(self.0.as_raw_fd()) {
                    Ok(fd) => Ok(StratisUnixRequest(AsyncFd::new(FdRef(fd))?)),
                    Err(e) => {
                        if let Some(errno) = e.as_errno() {
                            if errno == Errno::EAGAIN {
                                Err(io::Error::from(ErrorKind::WouldBlock))
                            } else {
                                Err(io::Error::new(
                                    ErrorKind::Other,
                                    format!("Failed with errno {}", errno),
                                ))
                            }
                        } else {
                            Err(io::Error::from(ErrorKind::Other))
                        }
                    }
                })
                .map_err(StratisError::from),
        ))
    }
}
