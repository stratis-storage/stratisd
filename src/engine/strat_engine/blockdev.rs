// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io::{Read, Write, ErrorKind, Seek, SeekFrom};
use std::fs::{OpenOptions, read_dir};
use std::path::{Path, PathBuf};
use std::io;
use std::str::FromStr;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use time::Timespec;
use devicemapper::Device;
use crc::crc32;
use uuid::Uuid;
use bytesize::ByteSize;

use types::{Sectors, SectorOffset};
use engine::{EngineResult, EngineError, ErrorEnum};

use consts::*;
use super::consts::*;

use super::metadata::SigBlock;
use super::metadata::validate_mda_size;

use super::util::blkdev_size;

pub use super::BlockDevSave;

type PoolUuid = Uuid;
type DevUuid = Uuid;

#[derive(Debug)]
enum DevOwnership {
    Ours(Uuid),
    Unowned,
    Theirs,
}

#[derive(Debug, Clone)]
pub struct BlockDev {
    pub dev: Device,
    pub devnode: PathBuf,
    pub sigblock: SigBlock,
}

impl BlockDev {
    /// Filter devices for admission to pool based on dev_infos.
    /// If there is an error finding out the info, return that error.
    /// Also, return an error if a device is not appropriate for this pool.
    fn filter_devs<I>(dev_infos: I,
                      pool_uuid: &Uuid,
                      force: bool)
                      -> EngineResult<Vec<(Device, (PathBuf, u64))>>
        where I: Iterator<Item = (Device, EngineResult<(PathBuf, u64, DevOwnership)>)>
    {
        let mut add_devs = Vec::new();
        for (dev, dev_result) in dev_infos {
            if dev_result.is_err() {
                return Err(dev_result.unwrap_err());
            }
            let (devnode, dev_size, ownership) = dev_result.unwrap();
            if dev_size < MIN_DEV_SIZE {
                let error_message = format!("{} too small, {} minimum",
                                            devnode.display(),
                                            ByteSize::b(MIN_DEV_SIZE as usize).to_string(true));
                return Err(EngineError::Stratis(ErrorEnum::Invalid(error_message)));
            };
            match ownership {
                DevOwnership::Unowned => add_devs.push((dev, (devnode, dev_size))),
                DevOwnership::Theirs => {
                    if !force {
                        let error_str = format!("First 4K of {} not zeroed", devnode.display());
                        return Err(EngineError::Stratis(ErrorEnum::Invalid(error_str)));
                    } else {
                        add_devs.push((dev, (devnode, dev_size)))
                    }
                }
                DevOwnership::Ours(uuid) => {
                    if *pool_uuid != uuid {
                        let error_str = format!("Device {} already belongs to Stratis pool {}",
                                                devnode.display(),
                                                uuid);
                        return Err(EngineError::Stratis(ErrorEnum::Invalid(error_str)));
                    }
                }
            }
        }
        Ok(add_devs)
    }

    /// Initialize multiple blockdevs at once. This allows all of them
    /// to be checked for usability before writing to any of them.
    pub fn initialize(pool_uuid: &PoolUuid,
                      devices: BTreeSet<Device>,
                      mda_size: Sectors,
                      force: bool)
                      -> EngineResult<BTreeMap<DevUuid, BlockDev>> {

        match validate_mda_size(mda_size) {
            None => {}
            Some(err) => {
                return Err(EngineError::Stratis(ErrorEnum::Invalid(err)));
            }
        };

        let dev_infos = devices.into_iter().map(|d: Device| (d, BlockDev::dev_info(&d)));

        let add_devs = try!(BlockDev::filter_devs(dev_infos, pool_uuid, force));

        let mut bds = BTreeMap::new();
        for (dev, (devnode, dev_size)) in add_devs {

            let dev_uuid = Uuid::new_v4();
            let bd = BlockDev {
                dev: dev,
                devnode: devnode,
                sigblock: SigBlock::new(pool_uuid,
                                        &dev_uuid,
                                        mda_size,
                                        Sectors(dev_size / SECTOR_SIZE)),
            };

            try!(bd.write_sigblock());
            bds.insert(dev_uuid, bd);
        }
        Ok(bds)
    }

    /// Gets device information, returns an error if problem with obtaining
    /// that information.
    fn dev_info(dev: &Device) -> EngineResult<(PathBuf, u64, DevOwnership)> {
        let devnode = try!(dev.path().ok_or_else(|| {
            io::Error::new(ErrorKind::InvalidInput,
                           format!("could not get device node from dev {}", dev.dstr()))
        }));
        let mut f = try!(OpenOptions::new()
            .read(true)
            .write(true)
            .open(&devnode)
            .map_err(|_| {
                io::Error::new(ErrorKind::PermissionDenied,
                               format!("Could not open {}", devnode.display()))
            }));

        let dev_size = try!(blkdev_size(&f));


        let mut ownership = DevOwnership::Unowned;

        let mut buf = [0u8; SECTOR_SIZE as usize];
        try!(f.seek(SeekFrom::Start(SECTOR_SIZE)));
        try!(f.read(&mut buf));

        if SigBlock::read_strat_magic(&buf, 0) == STRAT_MAGIC {
            ownership = match SigBlock::read_pool_uuid(&buf, 0) {
                Ok(pool_id) => DevOwnership::Ours(pool_id),
                Err(_) => {
                    let error_message = format!("Unable to read pool uuid for device {}",
                                                devnode.display());
                    return Err(EngineError::Stratis(ErrorEnum::Invalid(error_message)));
                }
            }
        } else {
            let mut buf = [0u8; 4096];
            try!(f.seek(SeekFrom::Start(0)));
            try!(f.read(&mut buf));
            if buf.iter().any(|x| *x != 0) {
                ownership = DevOwnership::Theirs;
            }
        }

        Ok((devnode, dev_size, ownership))
    }

    /// If a Path refers to a valid Stratis blockdev, return a BlockDev
    /// struct. Otherwise, return an error.
    pub fn setup(devnode: &Path) -> EngineResult<BlockDev> {
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

    pub fn to_save(&self) -> BlockDevSave {
        BlockDevSave {
            devnode: self.devnode.clone(),
            total_size: self.sigblock.total_size,
        }
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

            match BlockDev::setup(&devnode) {
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

    /// Size of the BDA copy at the beginning of the blockdev
    fn main_bda_size(&self) -> u64 {
        *(BDA_STATIC_HDR_SIZE + self.sigblock.mda_sectors + self.sigblock.reserved_sectors) *
        SECTOR_SIZE
    }

    /// Size of the BDA copy at the end of the blockdev
    fn aux_bda_size(&self) -> u64 {
        *(BDA_STATIC_HDR_SIZE + self.sigblock.mda_sectors) * SECTOR_SIZE
    }

    // Read metadata from newest MDA
    pub fn read_mdax(&self) -> EngineResult<Vec<u8>> {
        let younger_mda = match self.sigblock.mda.most_recent() {
            None => {
                let message = "Neither MDA region is in use";
                return Err(EngineError::Stratis(ErrorEnum::Invalid(message.into())));
            }
            Some(mda) => mda,
        };

        let mut f = try!(OpenOptions::new().read(true).open(&self.devnode));
        let mut buf = vec![0; younger_mda.used as usize];

        // read metadata from disk
        try!(f.seek(SeekFrom::Start((*BDA_STATIC_HDR_SIZE + *younger_mda.offset) * SECTOR_SIZE)));
        try!(f.read_exact(&mut buf));

        if younger_mda.crc != crc32::checksum_ieee(&buf) {
            return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput, "MDA CRC failed")));
            // TODO: Read end-of-blockdev copy
        }

        Ok(buf)
    }

    // Write metadata to least-recently-written MDA
    fn write_mdax(&mut self, time: &Timespec, metadata: &[u8]) -> EngineResult<()> {
        let aux_bda_size = self.aux_bda_size() as i64;
        let older_mda = self.sigblock.mda.least_recent();

        if metadata.len() > older_mda.length as usize {
            return Err(EngineError::Io(io::Error::new(io::ErrorKind::InvalidInput,
                                                      format!("Metadata too large for MDA, {} \
                                                               bytes",
                                                              metadata.len()))));
        }

        older_mda.crc = crc32::checksum_ieee(metadata);
        older_mda.length = metadata.len() as u32;
        older_mda.last_updated = *time;

        let mut f = try!(OpenOptions::new().write(true).open(&self.devnode));

        // write metadata to disk
        try!(f.seek(SeekFrom::Start((*BDA_STATIC_HDR_SIZE + *older_mda.offset) * SECTOR_SIZE)));
        try!(f.write_all(&metadata));
        try!(f.seek(SeekFrom::End(-aux_bda_size)));
        try!(f.seek(SeekFrom::Current((*older_mda.offset * SECTOR_SIZE) as i64)));
        try!(f.write_all(&metadata));
        try!(f.flush());

        Ok(())
    }

    fn write_sigblock(&self) -> EngineResult<()> {
        let mut buf = [0u8; SECTOR_SIZE as usize];
        self.sigblock.write(&mut buf, 0);
        try!(self.write_hdr_buf(&self.devnode, &buf));
        Ok(())
    }

    pub fn wipe_sigblock(&mut self) -> EngineResult<()> {
        let buf = [0u8; SECTOR_SIZE as usize];
        try!(self.write_hdr_buf(&self.devnode, &buf));
        Ok(())
    }

    fn write_hdr_buf(&self, devnode: &Path, buf: &[u8; SECTOR_SIZE as usize]) -> EngineResult<()> {
        let mut f = try!(OpenOptions::new().write(true).open(devnode));
        let zeroed = [0u8; (SECTOR_SIZE * 8) as usize];

        // Write 4K header to head & tail. Sigblock goes in sector 1.
        try!(f.write_all(&zeroed[..SECTOR_SIZE as usize]));
        try!(f.write_all(buf));
        try!(f.write_all(&zeroed[(SECTOR_SIZE * 2) as usize..]));
        try!(f.seek(SeekFrom::End(-(self.aux_bda_size() as i64))));
        try!(f.write_all(&zeroed[..SECTOR_SIZE as usize]));
        try!(f.write_all(buf));
        try!(f.write_all(&zeroed[(SECTOR_SIZE * 2) as usize..]));
        try!(f.flush());

        Ok(())
    }

    pub fn save_state(&mut self, time: &Timespec, metadata: &[u8]) -> EngineResult<()> {
        try!(self.write_mdax(time, metadata));
        try!(self.write_sigblock());

        Ok(())
    }

    /// Get the "x:y" device string for this blockdev
    pub fn dstr(&self) -> String {
        self.dev.dstr()
    }

    /// List the available-for-upper-layer-use range in this blockdev.
    pub fn avail_range(&self) -> (SectorOffset, Sectors) {
        let start = SectorOffset(*BDA_STATIC_HDR_SIZE + *self.sigblock.mda_sectors +
                                 *self.sigblock.reserved_sectors);
        let length = Sectors(*self.sigblock.total_size - *start - *BDA_STATIC_HDR_SIZE -
                             *self.sigblock.mda_sectors);
        (start, length)
    }
}
