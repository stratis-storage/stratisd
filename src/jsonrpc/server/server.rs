// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::remove_file,
    future::Future,
    os::unix::io::{AsRawFd, RawFd},
    path::Path,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::ready;
use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    sys::{
        socket::{
            accept, bind, listen, recvmsg, sendmsg, socket, AddressFamily, ControlMessageOwned,
            MsgFlags, SockAddr, SockFlag, SockType,
        },
        uio::IoVec,
    },
    unistd::close,
};
use tokio::{
    io::unix::AsyncFd,
    stream::{Stream, StreamExt},
    sync::Mutex,
    task::JoinHandle,
};

use crate::{
    engine::Engine,
    jsonrpc::{
        consts::{OP_ERR, RPC_SOCKADDR},
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
            StratisParamType::KeyList => {
                let (key_descs, rc, rs) =
                    stratis_result_to_return(key::key_list(engine).await, Vec::new());
                StratisRet::KeyList(key_descs, rc, rs)
            }
        }
    }
}

pub struct StratisServer {
    engine: Arc<Mutex<dyn Engine>>,
    listener: StratisUnixListener,
}

impl StratisServer {
    pub fn new<P>(engine: Arc<Mutex<dyn Engine>>, path: P) -> StratisResult<StratisServer>
    where
        P: AsRef<Path>,
    {
        Ok(StratisServer {
            engine,
            listener: StratisUnixListener::bind(path)?,
        })
    }

    async fn handle_request(&mut self) -> StratisResult<Option<()>> {
        let request_handler = match self.listener.next().await {
            Some(req_res) => req_res?,
            None => return Ok(None),
        };
        let engine = Arc::clone(&self.engine);
        tokio::spawn(async move {
            let fd = Arc::clone(&request_handler.fd);
            let params = match request_handler.await {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to receive request from connection: {}", e);
                    return;
                }
            };
            let ret = params.process(engine).await;
            if let Err(e) = StratisUnixResponse::new(fd, ret).await {
                warn!("Failed to respond to request: {}", e);
            }
        });
        Ok(Some(()))
    }

    pub async fn run(mut self) {
        loop {
            match self.handle_request().await {
                Ok(Some(())) => (),
                Ok(None) => {
                    info!("Unix socket listener can no longer accept connections; exiting...");
                    return;
                }
                Err(e) => {
                    warn!("Encountered an error while handling request: {}", e);
                }
            }
        }
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

fn try_recvmsg(fd: RawFd) -> StratisResult<StratisParams> {
    let mut cmsg_space = cmsg_space!([RawFd; 1]);
    let mut vec = vec![0; 65536];
    let rmsg = recvmsg(
        fd,
        &[IoVec::from_mut_slice(vec.as_mut_slice())],
        Some(&mut cmsg_space),
        MsgFlags::empty(),
    )?;

    let cmsgs = rmsg.cmsgs().collect();
    let fd_opt = handle_cmsgs(cmsgs)?;
    vec.truncate(rmsg.bytes);
    Ok(serde_json::from_slice(vec.as_slice())
        .map(|type_: StratisParamType| StratisParams { type_, fd_opt })?)
}

fn try_sendmsg(fd: RawFd, ret: &StratisRet) -> StratisResult<()> {
    let vec = serde_json::to_vec(ret)?;
    sendmsg(
        fd,
        &[IoVec::from_slice(vec.as_slice())],
        &[],
        MsgFlags::empty(),
        None,
    )?;
    Ok(())
}

pub struct StratisUnixRequest {
    fd: Arc<AsyncFd<FdRef>>,
}

impl Future for StratisUnixRequest {
    type Output = StratisResult<StratisParams>;

    fn poll(self: Pin<&mut Self>, ctxt: &mut Context) -> Poll<StratisResult<StratisParams>> {
        let poll_res = ready!(self.fd.poll_read_ready(ctxt));
        let mut poll_guard = match poll_res {
            Ok(poll) => poll,
            Err(e) => return Poll::Ready(Err(StratisError::from(e))),
        };
        let res = try_recvmsg(self.fd.as_raw_fd());
        poll_guard.clear_ready();
        Poll::Ready(res)
    }
}

pub struct StratisUnixResponse {
    fd: Arc<AsyncFd<FdRef>>,
    ret: StratisRet,
}

impl StratisUnixResponse {
    pub fn new(fd: Arc<AsyncFd<FdRef>>, ret: StratisRet) -> StratisUnixResponse {
        StratisUnixResponse { fd, ret }
    }
}

impl Future for StratisUnixResponse {
    type Output = StratisResult<()>;

    fn poll(self: Pin<&mut Self>, ctxt: &mut Context) -> Poll<StratisResult<()>> {
        let poll_res = ready!(self.fd.poll_write_ready(ctxt));
        let mut poll_guard = match poll_res {
            Ok(poll) => poll,
            Err(e) => return Poll::Ready(Err(StratisError::from(e))),
        };
        let res = try_sendmsg(self.fd.as_raw_fd(), &self.ret);
        poll_guard.clear_ready();
        Poll::Ready(res)
    }
}

pub struct StratisUnixListener {
    fd: AsyncFd<FdRef>,
}

impl StratisUnixListener {
    pub fn bind<P>(path: P) -> StratisResult<StratisUnixListener>
    where
        P: AsRef<Path>,
    {
        let _ = remove_file(path.as_ref());
        let fd = socket(
            AddressFamily::Unix,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )?;
        let flags = OFlag::from_bits(fcntl(fd, FcntlArg::F_GETFL)?).ok_or_else(|| {
            StratisError::Error("Unrecognized flag types returned from fcntl".to_string())
        })?;
        fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
        bind(fd, &SockAddr::new_unix(path.as_ref())?)?;
        listen(fd, 0)?;
        Ok(StratisUnixListener {
            fd: AsyncFd::new(FdRef::new(fd))?,
        })
    }
}

fn try_accept(fd: RawFd) -> StratisResult<StratisUnixRequest> {
    let fd = accept(fd)?;
    let flags = OFlag::from_bits(fcntl(fd, FcntlArg::F_GETFL)?).ok_or_else(|| {
        StratisError::Error("Unrecognized flag types returned from fcntl".to_string())
    })?;
    fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
    Ok(StratisUnixRequest {
        fd: Arc::new(AsyncFd::new(FdRef::new(fd))?),
    })
}

impl Stream for StratisUnixListener {
    type Item = StratisResult<StratisUnixRequest>;

    fn poll_next(
        self: Pin<&mut Self>,
        ctxt: &mut Context,
    ) -> Poll<Option<StratisResult<StratisUnixRequest>>> {
        let poll_res = ready!(self.fd.poll_read_ready(ctxt));
        let mut poll_guard = match poll_res {
            Ok(poll) => poll,
            Err(e) => return Poll::Ready(Some(Err(StratisError::from(e)))),
        };

        let opt = Some(try_accept(self.fd.as_raw_fd()));
        poll_guard.clear_ready();
        Poll::Ready(opt)
    }
}

pub fn run_server(engine: Arc<Mutex<dyn Engine>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        match StratisServer::new(engine, RPC_SOCKADDR) {
            Ok(server) => server.run().await,
            Err(e) => {
                error!("Failed to start stratisd-min server: {}", e);
            }
        }
    })
}
