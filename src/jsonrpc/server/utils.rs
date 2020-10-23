// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    future::Future,
    io::{self, ErrorKind},
    os::unix::io::{AsRawFd, RawFd},
    path::Path,
    pin::Pin,
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
use tokio::{io::unix::AsyncFd, stream::Stream};

use crate::{
    jsonrpc::consts::{OP_ERR, OP_OK, OP_OK_STR},
    stratis::{StratisError, StratisResult},
};

#[macro_export]
macro_rules! default_handler {
    ($respond:expr, $fn:path, $engine:expr, $default_value:expr $(, $args:expr)*) => {
        $respond.ok($crate::jsonrpc::server::utils::stratis_result_to_return(
            $fn(
                $engine,
                $($args),*
            ).await,
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

pub struct UnixFdReceiver(AsyncFd<FdRef>);

impl Future for UnixFdReceiver {
    type Output = StratisResult<FdRef>;

    fn poll(self: Pin<&mut Self>, ctxt: &mut Context) -> Poll<StratisResult<FdRef>> {
        let poll_res = ready!(self.0.poll_read_ready(ctxt));
        let mut poll_guard = match poll_res {
            Ok(poll) => poll,
            Err(e) => return Poll::Ready(Err(StratisError::from(e))),
        };
        Poll::Ready(
            poll_guard
                .with_io(|| {
                    let mut cmsg_space = cmsg_space!([RawFd; 1]);
                    let r_msg = recvmsg(
                        self.0.as_raw_fd(),
                        &[IoVec::from_mut_slice(&mut [0, 0, 0, 0])],
                        Some(&mut cmsg_space),
                        MsgFlags::empty(),
                    )
                    .map_err(|e| {
                        if let Some(errno) = e.as_errno() {
                            if errno == Errno::EAGAIN {
                                io::Error::from(ErrorKind::WouldBlock)
                            } else {
                                io::Error::new(
                                    ErrorKind::Other,
                                    format!("Failed with errno {}", errno),
                                )
                            }
                        } else {
                            io::Error::from(ErrorKind::Other)
                        }
                    })?;
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
                        Err(io::Error::from(ErrorKind::InvalidInput))
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
                                    Err(io::Error::from(ErrorKind::InvalidInput))
                                } else {
                                    Ok(FdRef(vec.pop().expect("Length is 1")))
                                }
                            }
                            _ => Err(io::Error::from(ErrorKind::InvalidInput)),
                        }
                    }
                })
                .map_err(StratisError::from),
        )
    }
}

pub struct UnixListenerStream(AsyncFd<FdRef>);

impl UnixListenerStream {
    pub fn bind<P>(path: P) -> StratisResult<UnixListenerStream>
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
        Ok(UnixListenerStream(AsyncFd::new(FdRef::new(fd))?))
    }
}

impl Stream for UnixListenerStream {
    type Item = StratisResult<UnixFdReceiver>;

    fn poll_next(
        self: Pin<&mut Self>,
        ctxt: &mut Context,
    ) -> Poll<Option<StratisResult<UnixFdReceiver>>> {
        let poll_res = ready!(self.0.poll_read_ready(ctxt));
        let mut poll_guard = match poll_res {
            Ok(poll) => poll,
            Err(e) => return Poll::Ready(Some(Err(StratisError::from(e)))),
        };

        Poll::Ready(Some(
            poll_guard
                .with_io(|| match accept(self.0.as_raw_fd()) {
                    Ok(fd) => Ok(UnixFdReceiver(AsyncFd::new(FdRef(fd))?)),
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
