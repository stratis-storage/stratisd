// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io::{Read, ErrorKind, Seek, SeekFrom};
use std::fs::{OpenOptions, read_dir};
use std::path::Path;
use std::io;
use std::str::FromStr;
use std::collections::BTreeMap;

use devicemapper::Device;
use uuid::Uuid;

use types::Sectors;
use engine::{EngineResult, EngineError, ErrorEnum};

use consts::SECTOR_SIZE;

use super::blockdev::BlockDev;
use super::metadata::SigBlock;
use super::util::blkdev_size;

type PoolUuid = Uuid;
type DevUuid = Uuid;


/// If a Path refers to a valid Stratis blockdev, return a BlockDev
/// struct. Otherwise, return an error.
fn setup(devnode: &Path) -> EngineResult<BlockDev> {
    let dev = try!(Device::from_str(&devnode.to_string_lossy()));

    let mut f = try!(OpenOptions::new()
        .read(true)
        .open(devnode)
        .map_err(|_| {
            io::Error::new(ErrorKind::PermissionDenied,
                           format!("Could not open {}", devnode.display()))
        }));

    let mut buf = [0u8; SECTOR_SIZE as usize];
    try!(f.seek(SeekFrom::Start(SECTOR_SIZE)));
    try!(f.read(&mut buf));

    Ok(BlockDev {
        dev: dev,
        devnode: devnode.to_owned(),
        sigblock: match SigBlock::read(&buf, 0, Sectors(try!(blkdev_size(&f)) / SECTOR_SIZE)) {
            Ok(sigblock) => sigblock,
            Err(err) => {
                return Err(EngineError::Stratis(ErrorEnum::Invalid(err)));
            }
        },
    })
}


/// Find all Stratis Blockdevs.
///
/// Returns a map of pool uuids to maps of blockdev uuids to blockdevs.
pub fn find_all() -> EngineResult<BTreeMap<PoolUuid, BTreeMap<DevUuid, BlockDev>>> {
    let mut pool_map = BTreeMap::new();
    for dir_e in try!(read_dir("/dev")) {
        let devnode = match dir_e {
            Ok(d) => d.path(),
            Err(_) => continue,
        };

        match setup(&devnode) {
            Ok(blockdev) => {
                pool_map.entry(blockdev.sigblock.pool_uuid)
                    .or_insert_with(BTreeMap::new)
                    .insert(blockdev.sigblock.dev_uuid, blockdev);
            }
            Err(_) => continue,
        };
    }

    Ok(pool_map)
}
