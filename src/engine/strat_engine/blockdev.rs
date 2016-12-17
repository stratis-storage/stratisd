// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io::{Read, Write, ErrorKind, Seek, SeekFrom};
use std::fs::{OpenOptions, read_dir};
use std::path::{Path, PathBuf};
use std::io;
use std::str::{FromStr, from_utf8};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use time::Timespec;
use devicemapper::Device;
use crc::crc32;
use byteorder::{LittleEndian, ByteOrder};
use uuid::Uuid;
use bytesize::ByteSize;

use types::{Sectors, SectorOffset};
use engine::{EngineResult, EngineError, ErrorEnum};

use consts::*;
use super::consts::*;
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MDA {
    pub last_updated: Timespec,
    used: u32,
    length: u32,
    crc: u32,
    offset: SectorOffset, // From start of MDA, not BDA
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDev {
    pub dev: Device,
    pub devnode: PathBuf,
    pub sectors: Sectors,
    pub mdaa: MDA,
    pub mdab: MDA,
    mda_sectors: Sectors,
    reserved_sectors: Sectors,
}

impl BlockDev {
    /// Validate MDA size
    /// Return None if MDA size is fine, otherwise a message.
    fn validate_mda_size(size: Sectors) -> Option<String> {
        if *size % NUM_MDA_COPIES != 0 {
            let error_message = format!("MDA size {} is not divisible by number of copies \
                                        required {}",
                                        *size,
                                        NUM_MDA_COPIES);
            return Some(error_message);
        };

        if size < MIN_MDA_SIZE {
            let error_message = format!("MDA size {} is less than minimum ({})",
                                        *size,
                                        *MIN_MDA_SIZE);
            return Some(error_message);
        };
        None
    }

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

        match BlockDev::validate_mda_size(mda_size) {
            None => {}
            Some(err) => {
                return Err(EngineError::Stratis(ErrorEnum::Invalid(err)));
            }
        };

        let dev_infos = devices.into_iter().map(|d: Device| (d, BlockDev::dev_info(&d)));

        let add_devs = try!(BlockDev::filter_devs(dev_infos, pool_uuid, force));

        let mut bds = BTreeMap::new();
        for (dev, (devnode, dev_size)) in add_devs {

            let mut bd = BlockDev {
                dev: dev,
                devnode: devnode,
                sectors: Sectors(dev_size / SECTOR_SIZE),
                mdaa: MDA {
                    last_updated: Timespec::new(0, 0),
                    used: 0,
                    length: (*mda_size / NUM_MDA_COPIES * SECTOR_SIZE) as u32,
                    crc: 0,
                    offset: SectorOffset(0),
                },
                mdab: MDA {
                    last_updated: Timespec::new(0, 0),
                    used: 0,
                    length: (*mda_size / NUM_MDA_COPIES * SECTOR_SIZE) as u32,
                    crc: 0,
                    offset: SectorOffset(*mda_size / NUM_MDA_COPIES),
                },
                mda_sectors: mda_size,
                reserved_sectors: MDA_RESERVED_SIZE,
            };

            let dev_uuid = Uuid::new_v4();
            try!(bd.write_sigblock(pool_uuid, &dev_uuid));
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

        if &buf[4..20] == STRAT_MAGIC {
            let pool_id = try!(Uuid::parse_str(from_utf8(&buf[32..64]).unwrap())
                .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid pool uid")));
            ownership = DevOwnership::Ours(pool_id);
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

    /// If a Path refers to a valid Stratis blockdev, return its
    /// parent uuid, its blockdev uuid, and the Blockdev.
    pub fn setup(devnode: &Path) -> EngineResult<(PoolUuid, DevUuid, BlockDev)> {
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

        if &buf[4..20] != STRAT_MAGIC {
            return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                      format!("{} is not a Stratis blockdev",
                                                              devnode.display()))));
        }

        let crc = crc32::checksum_ieee(&buf[4..SECTOR_SIZE as usize]);
        if crc != LittleEndian::read_u32(&buf[..4]) {
            dbgp!("{} header CRC failed", devnode.display());
            return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                      format!("{} header CRC failed",
                                                              devnode.display()))));
            // TODO: Try to read end-of-disk copy
        }

        let pool_id = try!(Uuid::parse_str(from_utf8(&buf[32..64]).unwrap())
            .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid pool uid")));
        let dev_id = try!(Uuid::parse_str(from_utf8(&buf[64..96]).unwrap())
            .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid dev uuid")));

        let mda_size = Sectors(LittleEndian::read_u32(&buf[160..164]) as u64);

        match BlockDev::validate_mda_size(mda_size) {
            None => {}
            Some(err) => {
                return Err(EngineError::Stratis(ErrorEnum::Invalid(err)));
            }
        };

        Ok((pool_id,
            dev_id,
            BlockDev {
                dev: dev,
                devnode: devnode.to_owned(),
                sectors: Sectors(try!(blkdev_size(&f)) / SECTOR_SIZE),
                mdaa: MDA {
                    last_updated: Timespec::new(LittleEndian::read_u64(&buf[64..72]) as i64,
                                                LittleEndian::read_u32(&buf[72..76]) as i32),
                    used: LittleEndian::read_u32(&buf[76..80]),
                    length: (*mda_size / NUM_MDA_COPIES * SECTOR_SIZE) as u32,
                    crc: LittleEndian::read_u32(&buf[80..84]),
                    offset: SectorOffset(0),
                },
                mdab: MDA {
                    last_updated: Timespec::new(LittleEndian::read_u64(&buf[96..104]) as i64,
                                                LittleEndian::read_u32(&buf[104..108]) as i32),
                    used: LittleEndian::read_u32(&buf[108..112]),
                    length: (*mda_size / NUM_MDA_COPIES * SECTOR_SIZE) as u32,
                    crc: LittleEndian::read_u32(&buf[112..116]),
                    offset: SectorOffset(*mda_size / NUM_MDA_COPIES),
                },
                mda_sectors: mda_size,
                reserved_sectors: Sectors(LittleEndian::read_u32(&buf[164..168]) as u64),
            }))
    }

    pub fn to_save(&self) -> BlockDevSave {
        BlockDevSave {
            devnode: self.devnode.clone(),
            sectors: self.sectors,
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
                Ok((pool_uuid, dev_uuid, blockdev)) => {
                    pool_map.entry(pool_uuid)
                        .or_insert_with(BTreeMap::new)
                        .insert(dev_uuid, blockdev);
                }
                Err(_) => continue,
            };
        }

        Ok(pool_map)
    }

    /// Size of the BDA copy at the beginning of the blockdev
    fn main_bda_size(&self) -> u64 {
        *(BDA_STATIC_HDR_SIZE + self.mda_sectors + self.reserved_sectors) * SECTOR_SIZE
    }

    /// Size of the BDA copy at the end of the blockdev
    fn aux_bda_size(&self) -> u64 {
        *(BDA_STATIC_HDR_SIZE + self.mda_sectors) * SECTOR_SIZE
    }

    // Read metadata from newest MDA
    pub fn read_mdax(&self) -> EngineResult<Vec<u8>> {
        let younger_mda = match self.mdaa.last_updated.cmp(&self.mdab.last_updated) {
            Ordering::Less => &self.mdab,
            Ordering::Greater => &self.mdaa,
            Ordering::Equal => &self.mdab,
        };

        if younger_mda.last_updated == Timespec::new(0, 0) {
            return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                      "Neither MDA region is in use")));
        }

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
        let older_mda = match self.mdaa.last_updated.cmp(&self.mdab.last_updated) {
            Ordering::Less => &mut self.mdaa,
            Ordering::Greater => &mut self.mdab,
            Ordering::Equal => &mut self.mdaa,
        };

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

    fn write_sigblock(&mut self, pool_uuid: &PoolUuid, dev_uuid: &DevUuid) -> EngineResult<()> {
        let mut buf = [0u8; SECTOR_SIZE as usize];
        buf[4..20].clone_from_slice(STRAT_MAGIC);
        LittleEndian::write_u64(&mut buf[20..28], *self.sectors);
        // no flags yet
        buf[32..64].clone_from_slice(pool_uuid.simple().to_string().as_bytes());
        buf[64..96].clone_from_slice(dev_uuid.simple().to_string().as_bytes());

        LittleEndian::write_u64(&mut buf[64..72], self.mdaa.last_updated.sec as u64);
        LittleEndian::write_u32(&mut buf[72..76], self.mdaa.last_updated.nsec as u32);
        LittleEndian::write_u32(&mut buf[76..80], self.mdaa.length);
        LittleEndian::write_u32(&mut buf[80..84], self.mdaa.crc);

        LittleEndian::write_u64(&mut buf[96..104], self.mdab.last_updated.sec as u64);
        LittleEndian::write_u32(&mut buf[104..108], self.mdab.last_updated.nsec as u32);
        LittleEndian::write_u32(&mut buf[108..112], self.mdab.length);
        LittleEndian::write_u32(&mut buf[112..116], self.mdab.crc);

        LittleEndian::write_u32(&mut buf[160..164], *self.mda_sectors as u32);
        LittleEndian::write_u32(&mut buf[164..168], *self.reserved_sectors as u32);

        // All done, calc CRC and write
        let hdr_crc = crc32::checksum_ieee(&buf[4..SECTOR_SIZE as usize]);
        LittleEndian::write_u32(&mut buf[..4], hdr_crc);

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

    pub fn save_state(&mut self,
                      time: &Timespec,
                      metadata: &[u8],
                      pool_uuid: &PoolUuid,
                      dev_uuid: &DevUuid)
                      -> EngineResult<()> {
        try!(self.write_mdax(time, metadata));
        try!(self.write_sigblock(pool_uuid, dev_uuid));

        Ok(())
    }

    /// Get the "x:y" device string for this blockdev
    pub fn dstr(&self) -> String {
        self.dev.dstr()
    }

    /// List the available-for-upper-layer-use range in this blockdev.
    pub fn avail_range(&self) -> (SectorOffset, Sectors) {
        let start = SectorOffset(*BDA_STATIC_HDR_SIZE + *self.mda_sectors + *self.reserved_sectors);
        let length = Sectors(*self.sectors - *start - *BDA_STATIC_HDR_SIZE - *self.mda_sectors);
        (start, length)
    }
}
