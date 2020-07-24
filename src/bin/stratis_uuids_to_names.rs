// Copyright 2020 Red Hat, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{
    collections::HashMap,
    env::args,
    error::Error,
    fmt::{self, Debug, Display},
    os::unix::net::UnixDatagram,
    time::Duration,
};

use dbus::{
    arg::{RefArg, Variant},
    blocking::Connection,
    Path,
};
use lazy_static::lazy_static;
use regex::Regex;
use uuid::Uuid;

pub const DEV_LOG: &str = "/dev/log";
/// Syslog priority syntax
/// (3 (SYSTEM) << 3) | 6 (INFO)
pub const SYSTEM_DAEMON_INFO: &str = "<30>";
/// Syslog priority syntax
/// (3 (SYSTEM) << 3) | 3 (ERROR)
pub const SYSTEM_DAEMON_ERROR: &str = "<27>";
pub const STRATIS_BUS_NAME: &str = "org.storage.stratis2";
pub const STRATIS_MANAGER_OBJECT: &str = "/org/storage/stratis2";
pub const STRATIS_POOL_IFACE: &str = "org.storage.stratis2.pool.r1";
pub const STRATIS_FS_IFACE: &str = "org.storage.stratis2.filesystem";
pub const DBUS_OM_IFACE: &str = "org.freedesktop.DBus.ObjectManager";
lazy_static! {
    static ref TIMEOUT: Duration = Duration::new(5, 0);
}

struct StratisUdevError(Option<String>);

impl StratisUdevError {
    fn new<D>(display: D) -> StratisUdevError
    where
        D: Display,
    {
        StratisUdevError(Some(display.to_string()))
    }
}

impl<E> From<E> for StratisUdevError
where
    E: Error,
{
    fn from(e: E) -> StratisUdevError {
        StratisUdevError::new(e)
    }
}

impl Debug for StratisUdevError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Could not convert Stratis UUIDs to names.")?;
        if let Some(ref msg) = self.0 {
            write!(f, " {}", msg)
        } else {
            Ok(())
        }
    }
}

type GMORet =
    HashMap<Path<'static>, HashMap<String, HashMap<String, Variant<Box<dyn RefArg + 'static>>>>>;

fn get_managed_objects() -> Result<GMORet, StratisUdevError> {
    let connection = Connection::new_system()?;
    let proxy = connection.with_proxy(STRATIS_BUS_NAME, STRATIS_MANAGER_OBJECT, *TIMEOUT);
    Ok(proxy
        .method_call(DBUS_OM_IFACE, "GetManagedObjects", ())
        .map(|r: (GMORet,)| r.0)?)
}

fn udev_name_to_uuids(dm_name: &str) -> Result<Option<(Uuid, Uuid)>, StratisUdevError> {
    let regex = Regex::new("stratis-1-(?P<pool>[0-9a-f]{32})-thin-fs-(?P<fs>[0-9a-f]{32})")?;
    let captures = match regex.captures(dm_name) {
        Some(cap) => cap,
        None => return Ok(None),
    };
    let pool_uuid = captures
        .name("pool")
        .map(|pu| Uuid::parse_str(pu.as_str()).expect("Format validated by regex"));
    let fs_uuid = captures
        .name("fs")
        .map(|pu| Uuid::parse_str(pu.as_str()).expect("Format validated by regex"));
    Ok(pool_uuid.and_then(|pu| fs_uuid.map(|fu| (pu, fu))))
}

fn uuid_to_stratis_name(
    managed_objects: &GMORet,
    iface_name: &'static str,
    uuid: Uuid,
) -> Result<Option<String>, StratisUdevError> {
    let mut names: Vec<_> = managed_objects
        .values()
        .filter_map(|map| {
            map.get(iface_name).and_then(|submap| {
                if submap.get("Uuid").and_then(|uuid_val| uuid_val.as_str())
                    == Some(&uuid.to_simple_ref().to_string())
                {
                    submap
                        .get("Name")
                        .and_then(|name| name.as_str().map(|n| n.to_string()))
                } else {
                    None
                }
            })
        })
        .collect();

    if names.len() > 1 {
        Err(StratisUdevError::new(format!(
            "More than one device has the UUID {}",
            uuid.to_simple_ref()
        )))
    } else {
        Ok(names.pop())
    }
}

fn main_report_error() -> Result<Option<(String, String)>, StratisUdevError> {
    let mut args = args();
    let dm_name = match args.nth(1) {
        Some(dm_name) => dm_name,
        None => {
            return Err(StratisUdevError::new(
                "Thinly provisioned filesystem devicemapper name required as argument.",
            ));
        }
    };
    match args.next().as_deref() {
        Some(action) => {
            if action != "change" && action != "add" {
                return Ok(None);
            }
        }
        None => {
            return Err(StratisUdevError::new("udev action required as argument."));
        }
    };

    let managed_objects = get_managed_objects()?;

    if let Some((pool_uuid, fs_uuid)) = udev_name_to_uuids(&dm_name)? {
        let pool_name = uuid_to_stratis_name(&managed_objects, STRATIS_POOL_IFACE, pool_uuid)?
            .ok_or_else(|| StratisUdevError::new("Could not get pool name from UUID."))?;
        let fs_name = uuid_to_stratis_name(&managed_objects, STRATIS_FS_IFACE, fs_uuid)?
            .ok_or_else(|| StratisUdevError::new("Could not get filesystem name from UUID."))?;
        Ok(Some((pool_name, fs_name)))
    } else {
        Ok(None)
    }
}

fn main() -> Result<(), StratisUdevError> {
    let sock = UnixDatagram::unbound().map_err(StratisUdevError::new)?;
    match main_report_error() {
        Ok(Some((pool_name, fs_name))) => {
            sock.send_to(
                format!(
                    "{}Symlink /dev/stratis/{}/{} created.",
                    SYSTEM_DAEMON_INFO, pool_name, fs_name,
                )
                .as_bytes(),
                DEV_LOG,
            )
            .map_err(StratisUdevError::new)?;

            println!("{} {}", pool_name, fs_name);
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(e) => {
            sock.send_to(
                format!("{}{:?}", SYSTEM_DAEMON_ERROR, e).as_bytes(),
                DEV_LOG,
            )
            .map_err(StratisUdevError::new)?;
            Err(e)
        }
    }
}
