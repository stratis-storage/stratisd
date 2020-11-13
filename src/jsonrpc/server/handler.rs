// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{fs::remove_file, os::unix::io::AsRawFd, sync::Arc};

use jsonrpsee::{
    common::{Error as RPCError, ErrorCode},
    raw::RawServer,
};
use tokio::{
    sync::{
        mpsc::{channel, Receiver},
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
        server::{key, pool, report, server::FdRef, udev},
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
                Stratis::KeySet { respond, key_desc } => {
                    if let Some(fd) = recv.recv().await {
                        default_handler!(
                            respond,
                            key::key_set,
                            engine,
                            None,
                            &key_desc,
                            fd.as_raw_fd()
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
                    if prompt {
                        if let Some(ref fd) = recv.recv().await {
                            default_handler!(
                                respond,
                                pool::pool_unlock,
                                engine,
                                false,
                                pool_uuid,
                                Some(fd.as_raw_fd())
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

pub fn run_server(engine: Arc<Mutex<dyn Engine>>) -> JoinHandle<()> {
    let (_send, recv) = channel(16);
    tokio::spawn(server_loop(engine, recv))
}
