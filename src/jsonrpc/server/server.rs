// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::{create_dir_all, remove_file},
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
        consts::RPC_SOCKADDR,
        interface::{StratisParamType, StratisParams, StratisRet},
        server::{filesystem, key, pool, report, udev, utils::stratis_result_to_return},
    },
    stratis::{StratisError, StratisResult},
};

impl StratisParams {
    async fn process(self, engine: Arc<Mutex<dyn Engine>>) -> StratisRet {
        match self.type_ {
            StratisParamType::KeySet(key_desc) => {
                let fd = expects_fd!(self.fd_opt, KeySet, None, true);
                StratisRet::KeySet(stratis_result_to_return(
                    key::key_set(engine, &key_desc, fd).await,
                    None,
                ))
            }
            StratisParamType::KeyUnset(key_desc) => {
                expects_fd!(self.fd_opt, KeyUnset, false, false);
                StratisRet::KeyUnset(stratis_result_to_return(
                    key::key_unset(engine, &key_desc).await,
                    false,
                ))
            }
            StratisParamType::KeyList => {
                expects_fd!(self.fd_opt, KeyUnset, false, false);
                StratisRet::KeyList(stratis_result_to_return(
                    key::key_list(engine).await,
                    Vec::new(),
                ))
            }
            StratisParamType::PoolCreate(name, paths, key_desc_opt) => {
                expects_fd!(self.fd_opt, PoolCreate, false, false);
                let path_ref: Vec<_> = paths.iter().map(|p| p.as_path()).collect();
                StratisRet::PoolCreate(stratis_result_to_return(
                    pool::pool_create(engine, name.as_str(), path_ref.as_slice(), key_desc_opt)
                        .await,
                    false,
                ))
            }
            StratisParamType::PoolRename(name, new_name) => {
                expects_fd!(self.fd_opt, PoolRename, false, false);
                StratisRet::PoolRename(stratis_result_to_return(
                    pool::pool_rename(engine, name.as_str(), new_name.as_str()).await,
                    false,
                ))
            }
            StratisParamType::PoolAddData(name, paths) => {
                expects_fd!(self.fd_opt, PoolAddData, false, false);
                let path_ref: Vec<_> = paths.iter().map(|p| p.as_path()).collect();
                StratisRet::PoolInitCache(stratis_result_to_return(
                    pool::pool_add_data(engine, name.as_str(), path_ref.as_slice()).await,
                    false,
                ))
            }
            StratisParamType::PoolInitCache(name, paths) => {
                expects_fd!(self.fd_opt, PoolInitCache, false, false);
                let path_ref: Vec<_> = paths.iter().map(|p| p.as_path()).collect();
                StratisRet::PoolInitCache(stratis_result_to_return(
                    pool::pool_init_cache(engine, name.as_str(), path_ref.as_slice()).await,
                    false,
                ))
            }
            StratisParamType::PoolAddCache(name, paths) => {
                expects_fd!(self.fd_opt, PoolAddCache, false, false);
                let path_ref: Vec<_> = paths.iter().map(|p| p.as_path()).collect();
                StratisRet::PoolAddCache(stratis_result_to_return(
                    pool::pool_add_cache(engine, name.as_str(), path_ref.as_slice()).await,
                    false,
                ))
            }
            StratisParamType::PoolDestroy(name) => {
                expects_fd!(self.fd_opt, PoolDestroy, false, false);
                StratisRet::PoolDestroy(stratis_result_to_return(
                    pool::pool_destroy(engine, name.as_str()).await,
                    false,
                ))
            }
            StratisParamType::PoolUnlock(unlock_method, uuid) => {
                StratisRet::PoolUnlock(stratis_result_to_return(
                    pool::pool_unlock(engine, unlock_method, uuid, self.fd_opt).await,
                    false,
                ))
            }
            StratisParamType::PoolList => {
                if let Some(fd) = self.fd_opt {
                    if let Err(e) = close(fd) {
                        warn!(
                            "Failed to close file descriptor {}: {}; a file \
                            descriptor may have been leaked",
                            fd, e,
                        );
                    }
                }
                StratisRet::PoolList(pool::pool_list(engine).await)
            }
            StratisParamType::PoolIsEncrypted(uuid) => {
                expects_fd!(self.fd_opt, PoolIsEncrypted, false, false);
                StratisRet::PoolIsEncrypted(stratis_result_to_return(
                    pool::pool_is_encrypted(engine, uuid).await,
                    false,
                ))
            }
            StratisParamType::PoolIsLocked(uuid) => {
                expects_fd!(self.fd_opt, PoolIsLocked, false, false);
                StratisRet::PoolIsLocked(stratis_result_to_return(
                    pool::pool_is_locked(engine, uuid).await,
                    false,
                ))
            }
            StratisParamType::PoolIsBound(uuid) => {
                expects_fd!(self.fd_opt, PoolIsBound, false, false);
                StratisRet::PoolIsBound(stratis_result_to_return(
                    pool::pool_is_bound(engine, uuid).await,
                    false,
                ))
            }
            StratisParamType::FsCreate(pool_name, fs_name) => {
                expects_fd!(self.fd_opt, FsCreate, false, false);
                StratisRet::FsCreate(stratis_result_to_return(
                    filesystem::filesystem_create(engine, &pool_name, &fs_name).await,
                    false,
                ))
            }
            StratisParamType::FsList => {
                if let Some(fd) = self.fd_opt {
                    if let Err(e) = close(fd) {
                        warn!(
                            "Failed to close file descriptor {}: {}; a file \
                            descriptor may have been leaked",
                            fd, e,
                        );
                    }
                }
                StratisRet::FsList(filesystem::filesystem_list(engine).await)
            }
            StratisParamType::FsDestroy(pool_name, fs_name) => {
                expects_fd!(self.fd_opt, FsDestroy, false, false);
                StratisRet::FsDestroy(stratis_result_to_return(
                    filesystem::filesystem_destroy(engine, &pool_name, &fs_name).await,
                    false,
                ))
            }
            StratisParamType::FsRename(pool_name, fs_name, new_fs_name) => {
                expects_fd!(self.fd_opt, FsRename, false, false);
                StratisRet::FsRename(stratis_result_to_return(
                    filesystem::filesystem_rename(engine, &pool_name, &fs_name, &new_fs_name).await,
                    false,
                ))
            }
            StratisParamType::Report => {
                if let Some(fd) = self.fd_opt {
                    if let Err(e) = close(fd) {
                        warn!(
                            "Failed to close file descriptor {}: {}; a file \
                            descriptor may have been leaked",
                            fd, e,
                        );
                    }
                }
                StratisRet::Report(report::report(engine).await)
            }
            StratisParamType::Udev(dm_name) => {
                expects_fd!(self.fd_opt, Udev, None, false);
                StratisRet::Udev(stratis_result_to_return(
                    udev::udev(engine, dm_name.as_str()).await,
                    None,
                ))
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
        let server = StratisServer {
            engine,
            listener: StratisUnixListener::bind(path)?,
        };
        #[cfg(feature = "systemd_notify")]
        systemd::daemon::notify(false, [("READY", "1")].iter())?;
        Ok(server)
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
        let _ = create_dir_all(
            Path::new(RPC_SOCKADDR)
                .parent()
                .expect("Static path always has parent"),
        );
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
