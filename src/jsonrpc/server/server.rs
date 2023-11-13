// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(feature = "systemd_compat")]
use std::collections::HashMap;
use std::{
    fs::{create_dir_all, remove_file},
    future::Future,
    io::{IoSlice, IoSliceMut},
    os::unix::io::{AsRawFd, OwnedFd, RawFd},
    path::Path,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    ready,
    stream::{Stream, StreamExt},
};
use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    sys::socket::{
        accept, bind, listen, recvmsg, sendmsg, socket, AddressFamily, ControlMessageOwned,
        MsgFlags, SockFlag, SockType, UnixAddr,
    },
    unistd::close,
};
use serde::Serialize;
use tokio::{io::unix::AsyncFd, task::JoinHandle};

#[cfg(feature = "systemd_compat")]
use crate::systemd;
use crate::{
    engine::Engine,
    jsonrpc::{
        consts::RPC_SOCKADDR,
        interface::{IpcResult, StratisParamType, StratisParams, StratisRet},
        server::{filesystem, key, pool, report, utils::stratis_result_to_return},
    },
    stratis::{StratisError, StratisResult},
};

impl StratisParams {
    async fn process(self, engine: Arc<dyn Engine>) -> IpcResult<StratisRet> {
        match self.type_ {
            StratisParamType::KeySet(key_desc) => {
                let fd = expects_fd!(self.fd_opt, true);
                Ok(StratisRet::KeySet(stratis_result_to_return(
                    key::key_set(engine, &key_desc, fd).await,
                    None,
                )))
            }
            StratisParamType::KeyUnset(key_desc) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::KeyUnset(stratis_result_to_return(
                    key::key_unset(engine, &key_desc).await,
                    false,
                )))
            }
            StratisParamType::KeyList => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::KeyList(stratis_result_to_return(
                    key::key_list(engine).await,
                    Vec::new(),
                )))
            }
            StratisParamType::PoolCreate(name, paths, encryption_info) => {
                expects_fd!(self.fd_opt, false);
                let path_ref: Vec<_> = paths.iter().map(|p| p.as_path()).collect();
                Ok(StratisRet::PoolCreate(stratis_result_to_return(
                    pool::pool_create(
                        engine,
                        name.as_str(),
                        path_ref.as_slice(),
                        encryption_info.as_ref(),
                    )
                    .await,
                    false,
                )))
            }
            StratisParamType::PoolRename(name, new_name) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolRename(stratis_result_to_return(
                    pool::pool_rename(engine, name.as_str(), new_name.as_str()).await,
                    false,
                )))
            }
            StratisParamType::PoolAddData(name, paths) => {
                expects_fd!(self.fd_opt, false);
                let path_ref: Vec<_> = paths.iter().map(|p| p.as_path()).collect();
                Ok(StratisRet::PoolAddData(stratis_result_to_return(
                    pool::pool_add_data(engine, name.as_str(), path_ref.as_slice()).await,
                    false,
                )))
            }
            StratisParamType::PoolInitCache(name, paths) => {
                expects_fd!(self.fd_opt, false);
                let path_ref: Vec<_> = paths.iter().map(|p| p.as_path()).collect();
                Ok(StratisRet::PoolInitCache(stratis_result_to_return(
                    pool::pool_init_cache(engine, name.as_str(), path_ref.as_slice()).await,
                    false,
                )))
            }
            StratisParamType::PoolAddCache(name, paths) => {
                expects_fd!(self.fd_opt, false);
                let path_ref: Vec<_> = paths.iter().map(|p| p.as_path()).collect();
                Ok(StratisRet::PoolAddCache(stratis_result_to_return(
                    pool::pool_add_cache(engine, name.as_str(), path_ref.as_slice()).await,
                    false,
                )))
            }
            StratisParamType::PoolDestroy(name) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolDestroy(stratis_result_to_return(
                    pool::pool_destroy(engine, name.as_str()).await,
                    false,
                )))
            }
            StratisParamType::PoolStart(id, unlock_method) => {
                Ok(StratisRet::PoolStart(stratis_result_to_return(
                    pool::pool_start(engine, id, unlock_method, self.fd_opt).await,
                    false,
                )))
            }
            StratisParamType::PoolStop(id) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolStop(stratis_result_to_return(
                    pool::pool_stop(engine, id).await,
                    false,
                )))
            }
            StratisParamType::PoolList => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolList(pool::pool_list(engine).await))
            }
            StratisParamType::PoolBindKeyring(id, key_desc) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolBindKeyring(stratis_result_to_return(
                    pool::pool_bind_keyring(engine, id, &key_desc).await,
                    false,
                )))
            }
            StratisParamType::PoolBindClevis(id, pin, clevis_info) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolBindClevis(stratis_result_to_return(
                    pool::pool_bind_clevis(engine, id, &pin, &clevis_info).await,
                    false,
                )))
            }
            StratisParamType::PoolUnbindKeyring(id) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolUnbindKeyring(stratis_result_to_return(
                    pool::pool_unbind_keyring(engine, id).await,
                    false,
                )))
            }
            StratisParamType::PoolUnbindClevis(id) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolUnbindClevis(stratis_result_to_return(
                    pool::pool_unbind_clevis(engine, id).await,
                    false,
                )))
            }
            StratisParamType::PoolRebindKeyring(id, key_desc) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolRebindKeyring(stratis_result_to_return(
                    pool::pool_rebind_keyring(engine, id, key_desc).await,
                    false,
                )))
            }
            StratisParamType::PoolRebindClevis(id) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolRebindClevis(stratis_result_to_return(
                    pool::pool_rebind_clevis(engine, id).await,
                    false,
                )))
            }
            StratisParamType::PoolIsEncrypted(id) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolIsEncrypted(stratis_result_to_return(
                    pool::pool_is_encrypted(engine, id).await,
                    false,
                )))
            }
            StratisParamType::PoolIsStopped(id) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolIsStopped(stratis_result_to_return(
                    pool::pool_is_stopped(engine, id).await,
                    false,
                )))
            }
            StratisParamType::PoolIsBound(id) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolIsBound(stratis_result_to_return(
                    pool::pool_is_bound(engine, id).await,
                    false,
                )))
            }
            StratisParamType::PoolHasPassphrase(id) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolHasPassphrase(stratis_result_to_return(
                    pool::pool_has_passphrase(engine, id).await,
                    false,
                )))
            }
            StratisParamType::PoolClevisPin(id) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::PoolClevisPin(stratis_result_to_return(
                    pool::pool_clevis_pin(engine, id).await,
                    None,
                )))
            }
            StratisParamType::FsCreate(pool_name, fs_name) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::FsCreate(stratis_result_to_return(
                    filesystem::filesystem_create(engine, &pool_name, &fs_name).await,
                    false,
                )))
            }
            StratisParamType::FsList => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::FsList(
                    filesystem::filesystem_list(engine).await,
                ))
            }
            StratisParamType::FsDestroy(pool_name, fs_name) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::FsDestroy(stratis_result_to_return(
                    filesystem::filesystem_destroy(engine, &pool_name, &fs_name).await,
                    false,
                )))
            }
            StratisParamType::FsRename(pool_name, fs_name, new_fs_name) => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::FsRename(stratis_result_to_return(
                    filesystem::filesystem_rename(engine, &pool_name, &fs_name, &new_fs_name).await,
                    false,
                )))
            }
            StratisParamType::Report => {
                expects_fd!(self.fd_opt, false);
                Ok(StratisRet::Report(report::report(engine).await))
            }
        }
    }
}

pub struct StratisServer {
    engine: Arc<dyn Engine>,
    listener: StratisUnixListener,
}

impl StratisServer {
    pub fn new<P>(engine: Arc<dyn Engine>, path: P) -> StratisResult<StratisServer>
    where
        P: AsRef<Path>,
    {
        let server = StratisServer {
            engine,
            listener: StratisUnixListener::bind(path)?,
        };
        #[cfg(feature = "systemd_compat")]
        systemd::notify(
            false,
            [("READY", "1")]
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect::<HashMap<String, String>>(),
        )?;
        Ok(server)
    }

    async fn handle_request(&mut self) -> StratisResult<bool> {
        let request_handler = match self.listener.next().await {
            Some(req_res) => req_res?,
            None => return Ok(false),
        };
        let engine = self.engine.clone();
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
        Ok(true)
    }

    pub async fn run(mut self) {
        loop {
            match self.handle_request().await {
                Ok(true) => (),
                Ok(false) => {
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

fn handle_cmsgs(cmsgs: Vec<ControlMessageOwned>) -> StratisResult<Option<RawFd>> {
    let mut fds = cmsgs
        .into_iter()
        .filter_map(|msg| {
            if let ControlMessageOwned::ScmRights(vec) = msg {
                Some(vec)
            } else {
                None
            }
        })
        .flatten()
        .collect::<Vec<_>>();

    if fds.len() > 1 {
        fds.into_iter().for_each(|fd| {
            if let Err(e) = close(fd) {
                warn!(
                    "Failed to close file descriptor {}: {}; potential for leaked file descriptor",
                    fd, e
                );
            }
        });
        Err(StratisError::Msg(
            "Unix packet contained more than one file descriptor".to_string(),
        ))
    } else {
        Ok(fds.pop())
    }
}

fn try_recvmsg(fd: RawFd) -> StratisResult<StratisParams> {
    let mut cmsg_space = cmsg_space!([RawFd; 1]);
    let mut vec = vec![0; 65536];
    let (cmsgs, bytes) = {
        let mut iovecs = [IoSliceMut::new(vec.as_mut_slice())];
        let rmsg = recvmsg::<UnixAddr>(fd, &mut iovecs, Some(&mut cmsg_space), MsgFlags::empty())?;
        (rmsg.cmsgs().collect(), rmsg.bytes)
    };

    let fd_opt = handle_cmsgs(cmsgs)?;
    vec.truncate(bytes);
    serde_json::from_slice(vec.as_slice())
        .map(|type_| StratisParams { type_, fd_opt })
        .map_err(StratisError::from)
}

fn try_sendmsg<S>(fd: RawFd, ret: &S) -> StratisResult<()>
where
    S: Serialize,
{
    let vec = serde_json::to_vec(ret)?;
    sendmsg::<UnixAddr>(
        fd,
        &[IoSlice::new(vec.as_slice())],
        &[],
        MsgFlags::empty(),
        None,
    )?;
    Ok(())
}

pub struct StratisUnixRequest {
    fd: Arc<AsyncFd<RawFd>>,
}

impl Future for StratisUnixRequest {
    type Output = StratisResult<StratisParams>;

    fn poll(self: Pin<&mut Self>, ctxt: &mut Context<'_>) -> Poll<Self::Output> {
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
    fd: Arc<AsyncFd<RawFd>>,
    ret: IpcResult<StratisRet>,
}

impl StratisUnixResponse {
    pub fn new(fd: Arc<AsyncFd<RawFd>>, ret: IpcResult<StratisRet>) -> StratisUnixResponse {
        StratisUnixResponse { fd, ret }
    }
}

impl Future for StratisUnixResponse {
    type Output = StratisResult<()>;

    fn poll(self: Pin<&mut Self>, ctxt: &mut Context<'_>) -> Poll<StratisResult<()>> {
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
    fd: AsyncFd<OwnedFd>,
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
        let flags =
            OFlag::from_bits(fcntl(fd.as_raw_fd(), FcntlArg::F_GETFL)?).ok_or_else(|| {
                StratisError::Msg("Unrecognized flag types returned from fcntl".to_string())
            })?;
        fcntl(fd.as_raw_fd(), FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
        bind(fd.as_raw_fd(), &UnixAddr::new(path.as_ref())?)?;
        listen(&fd, 0)?;
        Ok(StratisUnixListener {
            fd: AsyncFd::new(fd)?,
        })
    }
}

fn try_accept(fd: RawFd) -> StratisResult<StratisUnixRequest> {
    let fd = accept(fd)?;
    let flags = OFlag::from_bits(fcntl(fd, FcntlArg::F_GETFL)?).ok_or_else(|| {
        StratisError::Msg("Unrecognized flag types returned from fcntl".to_string())
    })?;
    fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
    Ok(StratisUnixRequest {
        fd: Arc::new(AsyncFd::new(fd)?),
    })
}

impl Stream for StratisUnixListener {
    type Item = StratisResult<StratisUnixRequest>;

    fn poll_next(
        self: Pin<&mut Self>,
        ctxt: &mut Context<'_>,
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

pub fn run_server(engine: Arc<dyn Engine>) -> JoinHandle<()> {
    tokio::spawn(async move {
        match StratisServer::new(engine, RPC_SOCKADDR) {
            Ok(server) => server.run().await,
            Err(e) => {
                error!("Failed to start stratisd-min server: {}", e);
            }
        }
    })
}
