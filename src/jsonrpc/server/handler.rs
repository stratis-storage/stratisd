// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::jsonrpc::consts::{SOCKFD_ADDR, SOCKFD_ADDR_DIR};

use std::{
    fs::{create_dir_all, remove_file},
    net::SocketAddr,
    os::unix::io::AsRawFd,
};

use futures_util::stream::StreamExt;
use jsonrpsee::{
    common::{Error, ErrorCode},
    raw::RawServer,
    transport::http::HttpTransportServer,
};
use tokio::{
    runtime::Runtime,
    sync::mpsc::{channel, Receiver, Sender},
};

use crate::{
    default_handler,
    engine::StratEngine,
    jsonrpc::{
        consts::RPC_SOCKADDR,
        interface::Stratis,
        server::{
            key, pool, report, udev,
            utils::{FdRef, UnixListenerStream},
        },
    },
};

async fn server_loop(mut recv: Receiver<FdRef>) -> Result<(), String> {
    let transport = HttpTransportServer::bind(
        &RPC_SOCKADDR
            .parse::<SocketAddr>()
            .expect("Valid socket address"),
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut engine = StratEngine::initialize().map_err(|e| e.to_string())?;
    let mut server = RawServer::new(transport);
    loop {
        if let Ok(event) = Stratis::next_request(&mut server).await {
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
                            &mut engine,
                            None,
                            &key_desc,
                            fd.as_raw_fd(),
                            interactive
                        )
                    } else {
                        respond.err(Error::new(ErrorCode::InternalError)).await
                    }
                }
                Stratis::KeyUnset { respond, key_desc } => {
                    default_handler!(respond, key::key_unset, &mut engine, false, &key_desc)
                }
                Stratis::KeyList { respond } => {
                    default_handler!(respond, key::key_list, &mut engine, Vec::new())
                }
                Stratis::PoolCreate {
                    respond,
                    name,
                    blockdev_paths,
                    key_desc,
                } => default_handler!(
                    respond,
                    pool::pool_create,
                    &mut engine,
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
                } => default_handler!(
                    respond,
                    pool::pool_rename,
                    &mut engine,
                    false,
                    &name,
                    &new_name
                ),
                Stratis::PoolInitCache {
                    respond,
                    name,
                    blockdev_paths,
                } => default_handler!(
                    respond,
                    pool::pool_init_cache,
                    &mut engine,
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
                    &mut engine,
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
                    &mut engine,
                    false,
                    &name,
                    blockdev_paths
                        .iter()
                        .map(|p| p.as_path())
                        .collect::<Vec<_>>()
                        .as_slice()
                ),
                Stratis::PoolDestroy { respond, name } => {
                    default_handler!(respond, pool::pool_destroy, &mut engine, false, &name)
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
                                &mut engine,
                                false,
                                pool_uuid,
                                prompt.map(|b| (fd.as_raw_fd(), b))
                            )
                        } else {
                            respond.err(Error::new(ErrorCode::InternalError)).await
                        }
                    } else {
                        default_handler!(
                            respond,
                            pool::pool_unlock,
                            &mut engine,
                            false,
                            pool_uuid,
                            None
                        )
                    }
                }
                Stratis::PoolIsEncrypted { respond, pool_uuid } => default_handler!(
                    respond,
                    pool::pool_is_encrypted,
                    &mut engine,
                    false,
                    pool_uuid
                ),
                Stratis::PoolList { respond } => respond.ok(pool::pool_list(&mut engine)).await,
                Stratis::Report { respond } => respond.ok(report::report(&engine)).await,
                Stratis::Udev { respond, dm_name } => {
                    default_handler!(respond, udev::udev, &mut engine, None, &dm_name)
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
            warn!("{}", e);
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

pub fn run_server() {
    let (send, recv) = channel(16);
    let runtime = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            error!("{}", e);
            return;
        }
    };
    runtime.spawn(async { file_descriptor_listener(send).await });
    if let Err(e) = runtime.block_on(server_loop(recv)) {
        error!("{}", e);
    }
}
