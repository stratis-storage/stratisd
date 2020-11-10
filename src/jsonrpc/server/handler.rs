// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::jsonrpc::consts::{SOCKFD_ADDR, SOCKFD_ADDR_DIR};

use std::{
    fs::{create_dir_all, remove_file},
    os::unix::io::AsRawFd,
    sync::Arc,
};

use futures_util::stream::StreamExt;
use jsonrpsee::{
    common::{Error as RPCError, ErrorCode},
    raw::RawServer,
};
use tokio::{
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};

use crate::{
    default_handler,
    engine::Engine,
    jsonrpc::{
        consts::RPC_SOCKADDR,
        interface::Stratis,
        server::{
            key, pool, report, udev,
            utils::{FdRef, UnixListenerStream},
        },
        transport::UdsTransportServer,
    },
};

async fn server_loop(engine_ref: Arc<Mutex<dyn Engine>>, mut recv: Receiver<FdRef>) {
    let _ = remove_file(RPC_SOCKADDR);
    let transport = match UdsTransportServer::bind(RPC_SOCKADDR) {
        Ok(t) => t,
        Err(e) => {
            error!(
                "Failed to bind Unix socket to address {}: {}",
                RPC_SOCKADDR, e
            );
            return;
        }
    };
    let mut server = RawServer::new(transport);
    loop {
        if let Ok(event) = Stratis::next_request(&mut server).await {
            let engine = Arc::clone(&engine_ref);
            match event {
                Stratis::KeySet {
                    respond,
                    key_desc,
                    interactive,
                } => {
                    if let Some(fd) = recv.recv().await {
                        default_handler!(
                            respond,
                            key::key_set,
                            engine,
                            None,
                            &key_desc,
                            fd.as_raw_fd(),
                            interactive
                        )
                    } else {
                        respond.err(RPCError::new(ErrorCode::InternalError)).await
                    }
                }
                Stratis::KeyUnset { respond, key_desc } => {
                    default_handler!(respond, key::key_unset, engine, false, &key_desc)
                }
                Stratis::KeyList { respond } => {
                    default_handler!(respond, key::key_list, engine, Vec::new())
                }
                Stratis::PoolCreate {
                    respond,
                    name,
                    blockdev_paths,
                    key_desc,
                } => default_handler!(
                    respond,
                    pool::pool_create,
                    engine,
                    false,
                    &name,
                    blockdev_paths
                        .iter()
                        .map(|p| p.as_path())
                        .collect::<Vec<_>>()
                        .as_slice(),
                    key_desc
                ),
                Stratis::PoolRename {
                    respond,
                    name,
                    new_name,
                } => default_handler!(respond, pool::pool_rename, engine, false, &name, &new_name),
                Stratis::PoolInitCache {
                    respond,
                    name,
                    blockdev_paths,
                } => default_handler!(
                    respond,
                    pool::pool_init_cache,
                    engine,
                    false,
                    &name,
                    blockdev_paths
                        .iter()
                        .map(|p| p.as_path())
                        .collect::<Vec<_>>()
                        .as_slice()
                ),
                Stratis::PoolAddData {
                    respond,
                    name,
                    blockdev_paths,
                } => default_handler!(
                    respond,
                    pool::pool_add_data,
                    engine,
                    false,
                    &name,
                    blockdev_paths
                        .iter()
                        .map(|p| p.as_path())
                        .collect::<Vec<_>>()
                        .as_slice()
                ),
                Stratis::PoolAddCache {
                    respond,
                    name,
                    blockdev_paths,
                } => default_handler!(
                    respond,
                    pool::pool_add_cache,
                    engine,
                    false,
                    &name,
                    blockdev_paths
                        .iter()
                        .map(|p| p.as_path())
                        .collect::<Vec<_>>()
                        .as_slice()
                ),
                Stratis::PoolDestroy { respond, name } => {
                    default_handler!(respond, pool::pool_destroy, engine, false, &name)
                }
                Stratis::PoolUnlock {
                    respond,
                    pool_uuid,
                    prompt,
                } => {
                    if prompt.is_some() {
                        if let Some(ref fd) = recv.recv().await {
                            default_handler!(
                                respond,
                                pool::pool_unlock,
                                engine,
                                false,
                                pool_uuid,
                                prompt.map(|b| (fd.as_raw_fd(), b))
                            )
                        } else {
                            respond.err(RPCError::new(ErrorCode::InternalError)).await
                        }
                    } else {
                        default_handler!(respond, pool::pool_unlock, engine, false, pool_uuid, None)
                    }
                }
                Stratis::PoolIsEncrypted { respond, pool_uuid } => {
                    default_handler!(respond, pool::pool_is_encrypted, engine, false, pool_uuid)
                }
                Stratis::PoolList { respond } => respond.ok(pool::pool_list(engine).await).await,
                Stratis::Report { respond } => respond.ok(report::report(engine).await).await,
                Stratis::Udev { respond, dm_name } => {
                    default_handler!(respond, udev::udev, engine, None, &dm_name)
                }
            }
        }
    }
}

pub async fn file_descriptor_listener(sender: Sender<FdRef>) {
    let _ = remove_file(SOCKFD_ADDR);
    if let Err(e) = create_dir_all(SOCKFD_ADDR_DIR) {
        warn!("{}", e);
    }
    let mut listener = match UnixListenerStream::bind(SOCKFD_ADDR) {
        Ok(l) => l,
        Err(e) => {
            error!(
                "Failed to find Unix socket to address {}: {}",
                SOCKFD_ADDR, e
            );
            return;
        }
    };
    loop {
        match listener.next().await {
            Some(Ok(stream)) => {
                let fd = match stream.await {
                    Ok(f) => f,
                    Err(e) => {
                        warn!("Could not get file descriptor sent from client: {}", e);
                        continue;
                    }
                };
                if let Err(e) = sender.send(fd).await {
                    warn!("Could not sent file descriptor to engine thread: {}", e);
                }
            }
            Some(Err(e)) => warn!("{}", e),
            None => unreachable!(),
        }
    }
}

pub fn run_server(engine: Arc<Mutex<dyn Engine>>) -> (JoinHandle<()>, JoinHandle<()>) {
    let (send, recv) = channel(16);
    let fd_join = tokio::spawn(async { file_descriptor_listener(send).await });
    let server_join = tokio::spawn(server_loop(engine, recv));
    (fd_join, server_join)
}
