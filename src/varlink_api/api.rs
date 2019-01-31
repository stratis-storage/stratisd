// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::error::Error;
use std::io;
use std::io::Read;
use std::io::Write;
use std::net::Shutdown::Both;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::FromRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::SecondsFormat;
use nix::sys::socket::getsockopt;
use nix::sys::socket::sockopt::PeerCredentials;
use uuid::Uuid;

use varlink;
use varlink::ConnectionHandler;

use crate::varlink_api::org_storage_stratis1::BlockDev as VLBlockDev;
use crate::varlink_api::org_storage_stratis1::Filesystem as VLFilesystem;
use crate::varlink_api::org_storage_stratis1::Pool as VLPool;

use crate::varlink_api::org_storage_stratis1::{
    new, Call_BlockDevUserInfoSet, Call_FileSystemCreate, Call_FileSystemDestroy,
    Call_FileSystemNameSet, Call_FileSystemSnapshot, Call_PoolCacheAdd, Call_PoolCreate,
    Call_PoolDestroy, Call_PoolDevsAdd, Call_PoolNameSet, Call_Pools, Call_Version,
    VarlinkInterface,
};

use crate::engine::{filesystem_mount_path, BlockDevTier, Engine, RenameAction};
use crate::stratis::{ErrorEnum, StratisError, StratisResult, VERSION};
use devicemapper::Sectors;

macro_attr! {
    #[derive(Clone, Copy, Debug)]
    #[allow(non_camel_case_types)]
    pub enum VarlinkErrorEnum {
        OK,
        ERROR,

        ALREADY_EXISTS,
        BUSY,
        IO_ERROR,
        INTERNAL_ERROR,
        NIX_ERROR,
        NOTFOUND,
        INVALID_ARGUMENT,
    }
}

/// Get the i64 value of this VarlinkErrorEnum constructor.
impl From<VarlinkErrorEnum> for i64 {
    fn from(e: VarlinkErrorEnum) -> i64 {
        e as i64
    }
}

pub fn engine_to_varlink_err_tuple(err: &StratisError) -> (i64, String) {
    let error = match *err {
        StratisError::Error(_) => VarlinkErrorEnum::INTERNAL_ERROR,
        StratisError::Engine(ref e, _) => match *e {
            ErrorEnum::Error => VarlinkErrorEnum::ERROR,
            ErrorEnum::AlreadyExists => VarlinkErrorEnum::ALREADY_EXISTS,
            ErrorEnum::Busy => VarlinkErrorEnum::BUSY,
            ErrorEnum::Invalid => VarlinkErrorEnum::ERROR,
            ErrorEnum::NotFound => VarlinkErrorEnum::NOTFOUND,
        },
        StratisError::Io(_) => VarlinkErrorEnum::IO_ERROR,
        StratisError::Nix(_) => VarlinkErrorEnum::NIX_ERROR,
        StratisError::Uuid(_)
        | StratisError::Utf8(_)
        | StratisError::Serde(_)
        | StratisError::DM(_)
        | StratisError::Udev(_) => VarlinkErrorEnum::INTERNAL_ERROR,
    };
    (error.into(), err.description().to_owned())
}

struct MyOrgStorageStratis1 {
    engine: Arc<Mutex<Engine>>,
}

impl MyOrgStorageStratis1 {
    fn new(engine: Arc<Mutex<Engine>>) -> MyOrgStorageStratis1 {
        MyOrgStorageStratis1 { engine }
    }
}

impl VarlinkInterface for MyOrgStorageStratis1 {
    fn block_dev_user_info_set(
        &self,
        call: &mut Call_BlockDevUserInfoSet,
        pool_uuid: String,
        block_dev_uuid: String,
        user_info: String,
    ) -> varlink::Result<()> {
        if let Ok(uuid) = Uuid::parse_str(&pool_uuid) {
            if let Ok(dev_uuid) = Uuid::parse_str(&block_dev_uuid) {
                let mut engine = self.engine.lock().unwrap();

                if let Some((pool_name, p)) = engine.get_mut_pool(uuid) {
                    match p.set_blockdev_user_info(&pool_name, dev_uuid, Some(&user_info)) {
                        Ok(_) => call.reply(true),
                        Err(err) => {
                            let (rc, rs) = engine_to_varlink_err_tuple(&err);
                            call.reply_base_error(rc, rs)
                        }
                    }
                } else {
                    call.reply_base_error(
                        VarlinkErrorEnum::NOTFOUND as i64,
                        format!(" Pool with uuid {} not found!", pool_uuid),
                    )
                }
            } else {
                call.reply_base_error(
                    VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                    format!("block_dev_uuid invalid {}", block_dev_uuid),
                )
            }
        } else {
            call.reply_base_error(
                VarlinkErrorEnum::INVALID_ARGUMENT.into(),
                format!("Pool uuid invalid {}", pool_uuid),
            )
        }
    }

    fn file_system_create(
        &self,
        call: &mut Call_FileSystemCreate,
        pool_uuid: String,
        names: Vec<String>,
    ) -> varlink::Result<()> {
        if let Ok(uuid) = Uuid::parse_str(&pool_uuid) {
            let mut engine = self.engine.lock().unwrap();

            if let Some((pool_name, p)) = engine.get_mut_pool(uuid) {
                let filesystems = names
                    .iter()
                    .map(|x| (x.as_str(), None))
                    .collect::<Vec<(&str, Option<Sectors>)>>();

                match p.create_filesystems(uuid, &pool_name, &filesystems) {
                    Ok(ref created) => call.reply(created[0].1.to_string()),
                    Err(err) => {
                        let (rc, rs) = engine_to_varlink_err_tuple(&err);
                        call.reply_base_error(rc, rs)
                    }
                }
            } else {
                call.reply_base_error(
                    VarlinkErrorEnum::NOTFOUND as i64,
                    format!("Pool uuid {} not found!", pool_uuid),
                )
            }
        } else {
            call.reply_base_error(
                VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                format!("Pool uuid invalid {}", pool_uuid),
            )
        }
    }

    fn file_system_destroy(
        &self,
        call: &mut Call_FileSystemDestroy,
        pool_uuid: String,
        fs_uuid: Vec<String>,
    ) -> varlink::Result<()> {
        if let Ok(uuid) = Uuid::parse_str(&pool_uuid) {
            let mut fs_uuids = Vec::new();
            for fs_u in fs_uuid {
                match Uuid::parse_str(&fs_u) {
                    Ok(fs_uuid) => fs_uuids.push(fs_uuid),
                    Err(_) => {
                        return call.reply_base_error(
                            VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                            format!("FS uuid invalid {}", fs_u),
                        );
                    }
                }
            }

            let mut engine = self.engine.lock().unwrap();

            if let Some((pool_name, p)) = engine.get_mut_pool(uuid) {
                // Build array of fs to destroy
                let mut filesystems = Vec::new();
                for fs in fs_uuids {
                    if let Some((_, _)) = p.get_filesystem(fs) {
                        filesystems.push(fs);
                    } else {
                        return call.reply_base_error(
                            VarlinkErrorEnum::NOTFOUND as i64,
                            format!("Filesystem uuid {} not found in pool {}", fs, pool_name),
                        );
                    }
                }

                match p.destroy_filesystems(&pool_name, &filesystems) {
                    Ok(_) => call.reply(true),
                    Err(err) => {
                        let (rc, rs) = engine_to_varlink_err_tuple(&err);
                        call.reply_base_error(rc, rs)
                    }
                }
            } else {
                call.reply_base_error(
                    VarlinkErrorEnum::NOTFOUND as i64,
                    format!("Pool uuid {} not found!", pool_uuid),
                )
            }
        } else {
            call.reply_base_error(
                VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                format!("Pool uuid invalid {}", pool_uuid),
            )
        }
    }

    fn file_system_name_set(
        &self,
        call: &mut Call_FileSystemNameSet,
        pool_uuid: String,
        fs_uuid: String,
        name: String,
    ) -> varlink::Result<()> {
        if let Ok(uuid) = Uuid::parse_str(&pool_uuid) {
            if let Ok(fs) = Uuid::parse_str(&fs_uuid) {
                let mut engine = self.engine.lock().unwrap();

                if let Some((pool_name, p)) = engine.get_mut_pool(uuid) {
                    match p.rename_filesystem(&pool_name, fs, &name) {
                        Ok(RenameAction::NoSource) => {
                            let error_message = format!(
                                "pool {} - {} doesn't know about filesystem {}",
                                pool_uuid, pool_name, fs_uuid
                            );
                            call.reply_base_error(
                                VarlinkErrorEnum::INVALID_ARGUMENT.into(),
                                error_message,
                            )
                        }
                        Ok(RenameAction::Identity) => call.reply(false),
                        Ok(RenameAction::Renamed) => call.reply(true),
                        Err(err) => {
                            let (rc, rs) = engine_to_varlink_err_tuple(&err);
                            call.reply_base_error(rc, rs)
                        }
                    }
                } else {
                    call.reply_base_error(
                        VarlinkErrorEnum::NOTFOUND as i64,
                        format!("Pool uuid {} not found!", pool_uuid),
                    )
                }
            } else {
                call.reply_base_error(
                    VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                    format!("FS uuid invalid {}", fs_uuid),
                )
            }
        } else {
            call.reply_base_error(
                VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                format!("Pool uuid invalid {}", pool_uuid),
            )
        }
    }

    fn file_system_snapshot(
        &self,
        call: &mut Call_FileSystemSnapshot,
        pool_uuid: String,
        fs_uuid: String,
        name: String,
    ) -> varlink::Result<()> {
        if let Ok(uuid) = Uuid::parse_str(&pool_uuid) {
            if let Ok(fs) = Uuid::parse_str(&fs_uuid) {
                let mut engine = self.engine.lock().unwrap();

                if let Some((pool_name, p)) = engine.get_mut_pool(uuid) {
                    if let Some((_, _)) = p.get_filesystem(fs) {
                        match p.snapshot_filesystem(uuid, &pool_name, fs, &name) {
                            Ok((ss_uuid, _)) => call.reply(ss_uuid.to_simple_ref().to_string()),
                            Err(err) => {
                                let (rc, rs) = engine_to_varlink_err_tuple(&err);
                                call.reply_base_error(rc, rs)
                            }
                        }
                    } else {
                        call.reply_base_error(
                            VarlinkErrorEnum::NOTFOUND as i64,
                            format!("Filesystem uuid {} not found!", fs_uuid),
                        )
                    }
                } else {
                    call.reply_base_error(
                        VarlinkErrorEnum::NOTFOUND as i64,
                        format!("Pool uuid {} not found!", pool_uuid),
                    )
                }
            } else {
                call.reply_base_error(
                    VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                    format!("FS uuid invalid {}", fs_uuid),
                )
            }
        } else {
            call.reply_base_error(
                VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                format!("Pool uuid invalid {}", pool_uuid),
            )
        }
    }

    fn pool_cache_add(
        &self,
        call: &mut Call_PoolCacheAdd,
        pool_uuid: String,
        devices: Vec<String>,
    ) -> varlink::Result<()> {
        // TODO use same code for cache add and data add
        if let Ok(uuid) = Uuid::parse_str(&pool_uuid) {
            let mut engine = self.engine.lock().unwrap();

            if let Some((pool_name, p)) = engine.get_mut_pool(uuid) {
                let blockdevs = devices.iter().map(|x| Path::new(x)).collect::<Vec<&Path>>();
                match p.add_blockdevs(uuid, &pool_name, &blockdevs, BlockDevTier::Cache) {
                    Ok(uuids) => {
                        call.reply(uuids.iter().map(|u| u.to_string()).collect::<Vec<_>>())
                    }
                    Err(err) => {
                        let (rc, rs) = engine_to_varlink_err_tuple(&err);
                        call.reply_base_error(rc, rs)
                    }
                }
            } else {
                call.reply_base_error(
                    VarlinkErrorEnum::NOTFOUND as i64,
                    format!("Pool uuid {} not found!", pool_uuid),
                )
            }
        } else {
            call.reply_base_error(
                VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                format!("Pool uuid invalid {}", pool_uuid),
            )
        }
    }

    fn pool_create(
        &self,
        call: &mut Call_PoolCreate,
        name: String,
        redundancy: Option<i64>,
        devices: Vec<String>,
    ) -> varlink::Result<()> {
        let blockdevs = devices.iter().map(|x| Path::new(x)).collect::<Vec<&Path>>();

        let r = match redundancy {
            Some(r) => Some(r as u16),
            None => None,
        };

        match self
            .engine
            .lock()
            .unwrap()
            .create_pool(&name, &blockdevs, r)
        {
            Ok(pool_uuid) => call.reply(pool_uuid.to_string()),
            Err(err) => {
                let (rc, rs) = engine_to_varlink_err_tuple(&err);
                call.reply_base_error(rc, rs)
            }
        }
    }

    fn pool_destroy(&self, call: &mut Call_PoolDestroy, pool_uuid: String) -> varlink::Result<()> {
        if let Ok(uuid) = Uuid::parse_str(&pool_uuid) {
            match self.engine.lock().unwrap().destroy_pool(uuid) {
                Ok(action) => call.reply(action),
                Err(err) => {
                    let (rc, rs) = engine_to_varlink_err_tuple(&err);
                    call.reply_base_error(rc, rs)
                }
            }
        } else {
            call.reply_base_error(
                VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                format!("Pool uuid invalid {}", pool_uuid),
            )
        }
    }

    fn pool_devs_add(
        &self,
        call: &mut Call_PoolDevsAdd,
        pool_uuid: String,
        devices: Vec<String>,
    ) -> varlink::Result<()> {
        if let Ok(uuid) = Uuid::parse_str(&pool_uuid) {
            let mut engine = self.engine.lock().unwrap();

            if let Some((pool_name, p)) = engine.get_mut_pool(uuid) {
                let blockdevs = devices.iter().map(|x| Path::new(x)).collect::<Vec<&Path>>();
                match p.add_blockdevs(uuid, &pool_name, &blockdevs, BlockDevTier::Data) {
                    Ok(uuids) => {
                        call.reply(uuids.iter().map(|u| u.to_string()).collect::<Vec<_>>())
                    }
                    Err(err) => {
                        let (rc, rs) = engine_to_varlink_err_tuple(&err);
                        call.reply_base_error(rc, rs)
                    }
                }
            } else {
                call.reply_base_error(
                    VarlinkErrorEnum::NOTFOUND as i64,
                    format!("Pool uuid {} not found!", pool_uuid),
                )
            }
        } else {
            call.reply_base_error(
                VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                format!("Pool uuid invalid {}", pool_uuid),
            )
        }
    }

    fn pool_name_set(
        &self,
        call: &mut Call_PoolNameSet,
        pool_uuid: String,
        new_name: String,
    ) -> varlink::Result<()> {
        if let Ok(uuid) = Uuid::parse_str(&pool_uuid) {
            match self.engine.lock().unwrap().rename_pool(uuid, &new_name) {
                Ok(RenameAction::NoSource) => {
                    let error_message = format!("Pool {} not found!", &pool_uuid);
                    call.reply_base_error(VarlinkErrorEnum::NOTFOUND as i64, error_message)
                }
                Ok(RenameAction::Identity) => call.reply(false),
                Ok(RenameAction::Renamed) => call.reply(true),
                Err(err) => {
                    let (rc, rs) = engine_to_varlink_err_tuple(&err);
                    call.reply_base_error(rc, rs)
                }
            }
        } else {
            call.reply_base_error(
                VarlinkErrorEnum::INVALID_ARGUMENT as i64,
                format!("Pool uuid invalid {}", pool_uuid),
            )
        }
    }

    fn pools(&self, call: &mut Call_Pools) -> varlink::Result<()> {
        let mut result = vec![];
        let engine = self.engine.lock().unwrap();

        for (name, pool_uuid, p) in engine.pools() {
            // Build up the block devs
            let mut block_devs = vec![];
            for (uuid, b) in p.blockdevs() {
                let (tier_t, _) = p
                    .get_blockdev(uuid)
                    .expect("we just got uuid from blockdevs");
                let bd = VLBlockDev {
                    uuid: uuid.to_simple_ref().to_string(),
                    devnode: b.devnode().to_str().expect("unix is utf-8").to_string(),
                    hardware_info: b.hardware_info().unwrap_or(&String::from("")).to_string(),
                    initialization_time: format!("{}", b.initialization_time().timestamp()),
                    state: b.state() as i64,
                    total_physical_size: format!("{}", *b.size()),
                    tier: tier_t as i64,
                    user_info: b.user_info().unwrap_or(&String::from("")).to_string(),
                };
                block_devs.push(bd);
            }

            // Build up the filesystems
            let mut fs_list = vec![];
            for (fs_name, uuid, fs) in p.filesystems() {
                let vl_fs = VLFilesystem {
                    uuid: uuid.to_simple_ref().to_string(),
                    name: fs_name.to_string(),
                    // devnode: fs.devnode().to_str().expect("unix is utf-8").to_string(),
                    devnode: format!("{}", filesystem_mount_path(name.clone(), fs_name).display()),
                    used: (*fs.used().unwrap()).to_string(),
                    created: fs.created().to_rfc3339_opts(SecondsFormat::Secs, true),
                };
                fs_list.push(vl_fs);
            }

            let varlink_pool = VLPool {
                name: name.to_string(),
                uuid: pool_uuid.to_simple_ref().to_string(),
                total_physical_size: format!("{}", *p.total_physical_size()),
                total_pyhsical_used: format!("{}", *p.total_physical_used().unwrap()),
                extend_state: p.extend_state() as i64,
                space_state: p.free_space_state() as i64,
                state: p.state() as i64,
                block_devs,
                file_systems: fs_list,
            };

            result.push(varlink_pool);
        }

        call.reply(result)
    }
    fn version(&self, call: &mut Call_Version) -> varlink::Result<()> {
        call.reply(String::from(VERSION))
    }
}

fn abstract_bind<P: AsRef<Path>>(path: P) -> std::io::Result<UnixListener> {
    fn cvt(v: libc::c_int) -> std::io::Result<libc::c_int> {
        if v < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(v)
        }
    }

    unsafe fn sockaddr_un<P: AsRef<Path>>(
        path: P,
    ) -> std::io::Result<(libc::sockaddr_un, libc::socklen_t)> {
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        let base = &addr as *const _ as usize;
        let sun_path_offset = &addr.sun_path as *const _ as usize - base;
        let bytes = path.as_ref().as_os_str().as_bytes();

        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;

        if bytes[0] != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "first character in abstract path must be null byte \\0",
            ));
        }

        if bytes.len() > addr.sun_path.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path must be no longer than SUN_LEN",
            ));
        }

        for (dst, src) in addr.sun_path.iter_mut().zip(bytes.iter()) {
            *dst = *src as libc::c_char;
        }

        let len = sun_path_offset + bytes.len();
        Ok((addr, len as libc::socklen_t))
    }

    fn inner(path: &Path) -> std::io::Result<UnixListener> {
        unsafe {
            let raw_fd = cvt(libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0))?;
            let (addr, len) = sockaddr_un(path)?;
            cvt(libc::bind(raw_fd, &addr as *const _ as *const _, len as _))?;
            cvt(libc::listen(raw_fd, 128))?;
            Ok(UnixListener::from_raw_fd(raw_fd))
        }
    }
    inner(path.as_ref())
}

struct ConnectionEntry {
    stream: UnixStream,
    buffer: Vec<u8>,
}

pub struct StratisVarlinkService {
    listener_fd: i32,
    listener: UnixListener,
    service: varlink::VarlinkService,
    fdmap: HashMap<i32, ConnectionEntry>,
}

impl StratisVarlinkService {
    pub fn initialize(engine: Arc<Mutex<Engine>>) -> StratisResult<StratisVarlinkService> {
        let mystratis = MyOrgStorageStratis1::new(engine);
        let myinterface = new(Box::new(mystratis));

        let service = varlink::VarlinkService::new(
            "org.stratis",
            "Stratis service",
            "0.1",
            "https://github.com/stratis-storage/stratisd",
            vec![Box::new(myinterface)],
        );

        let listener = abstract_bind("\0stratis-storage1")?;
        listener.set_nonblocking(true)?;

        let listener_fd = listener.as_raw_fd();

        debug!("varlink listening on fd: {}", listener_fd);

        Ok(StratisVarlinkService {
            listener_fd,
            listener,
            service,
            fdmap: HashMap::new(),
        })
    }

    pub fn poll_fds(&mut self) -> Vec<libc::pollfd> {
        let mut fds = self
            .fdmap
            .iter()
            .map(|(k, _)| libc::pollfd {
                fd: *k,
                revents: 0,
                events: libc::POLLIN,
            })
            .collect::<Vec<libc::pollfd>>();

        fds.push(libc::pollfd {
            fd: self.listener_fd,
            revents: 0,
            events: libc::POLLIN,
        });

        fds
    }

    pub fn handle(&mut self, fds: &[libc::pollfd]) {
        let mut fds_to_remove = vec![];

        for pfd in fds.iter().filter(|pfd| pfd.revents != 0) {
            if pfd.fd == self.listener_fd {
                // New client
                if let Ok((client, c_addr)) = self.listener.accept() {
                    if client.set_nonblocking(true).is_ok() {
                        let fd = client.as_raw_fd();
                        match getsockopt(fd, PeerCredentials) {
                            Ok(who) => {
                                if who.uid() == 0 && who.gid() == 0 {
                                    debug!("Client connected: fd = {} {:?}", fd, c_addr);
                                    self.fdmap.insert(
                                        fd,
                                        ConnectionEntry {
                                            stream: client,
                                            buffer: Vec::new(),
                                        },
                                    );
                                } else {
                                    warn!(
                                        "Process: {} uid: {} gid: {} denied!",
                                        who.pid(),
                                        who.uid(),
                                        who.gid()
                                    );
                                    let _ = client.shutdown(Both);
                                }
                            }
                            Err(e) => {
                                error!("getsockopt failed: {:?}", e);
                                let _ = client.shutdown(Both);
                            }
                        }
                    }
                }
            } else {
                // Handle client socket activity
                if let Some(client) = self.fdmap.get_mut(&pfd.fd) {
                    loop {
                        let mut readbuf: [u8; 8192] = [0; 8192];

                        match client.stream.read(&mut readbuf) {
                            Ok(0) => {
                                let _ = client.stream.shutdown(Both);
                                fds_to_remove.push(pfd.fd);
                                break;
                            }
                            Ok(len) => {
                                let mut response: Vec<u8> = Vec::new();

                                client.buffer.append(&mut readbuf[0..len].to_vec());

                                debug!(
                                    "Handling: {}",
                                    String::from_utf8_lossy(&client.buffer.as_slice())
                                );

                                match self.service.handle(
                                    &mut client.buffer.as_slice(),
                                    &mut response,
                                    None,
                                ) {
                                    // TODO: buffer output and write only on POLLOUT
                                    Ok((unprocessed_bytes, _)) => {
                                        if !unprocessed_bytes.is_empty() {
                                            debug!(
                                                "Unprocessed bytes: {}",
                                                String::from_utf8_lossy(&unprocessed_bytes)
                                            );
                                        }
                                        client.buffer.clone_from(&unprocessed_bytes);

                                        if let Err(err) = client.stream.write(response.as_ref()) {
                                            debug!("write error: {}", err);
                                            let _ = client.stream.shutdown(Both);
                                            fds_to_remove.push(pfd.fd);
                                            break;
                                        }
                                    }
                                    Err(e) => match e.kind() {
                                        err => {
                                            debug!("handler error: {}", err);
                                            let _ = client.stream.shutdown(Both);
                                            fds_to_remove.push(pfd.fd);
                                            break;
                                        }
                                    },
                                }
                            }
                            Err(e) => match e.kind() {
                                io::ErrorKind::WouldBlock => {
                                    break;
                                }
                                _ => {
                                    let _ = client.stream.shutdown(Both);
                                    fds_to_remove.push(pfd.fd);
                                    debug!("IO error: {}", e);
                                    break;
                                }
                            },
                        }
                    }
                }
            }
        }

        for i in fds_to_remove {
            debug!("Client with fd {} gone!", i);
            self.fdmap.remove(&i);
        }
    }
}
