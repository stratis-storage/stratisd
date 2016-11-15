// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io::{Read, Write, ErrorKind, Seek, SeekFrom};
use std::fs::{OpenOptions, read_dir};
use std::path::{Path, PathBuf};
use std::io;
use std::str::{FromStr, from_utf8};
use std::cmp::Ordering;

use time::Timespec;
use devicemapper::Device;
use crc::crc32;
use byteorder::{LittleEndian, ByteOrder};
use uuid::Uuid;
use bytesize::ByteSize;

use types::{Sectors, SectorOffset};
use engine::{EngineResult, EngineError};

use consts::*;
use super::consts::*;
use super::util::blkdev_size;

pub use super::BlockDevSave;

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
    pub pool_uuid: Uuid,
    pub dev: Device,
    pub dev_uuid: Uuid,
    pub path: PathBuf,
    pub sectors: Sectors,
    pub mdaa: MDA,
    pub mdab: MDA,
    mda_sectors: Sectors,
    reserved_sectors: Sectors,
}

impl BlockDev {
    /// Initialize multiple blockdevs at once. This allows all of them
    /// to be checked for usability before writing to any of them.
    pub fn initialize(pool_uuid: &Uuid,
                      paths: &[&Path],
                      mda_size: Sectors,
                      force: bool)
                      -> EngineResult<Vec<BlockDev>> {

        if *mda_size % 2 != 0 {
            return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                      format!("mda size {} is not an even \
                                                               number",
                                                              *mda_size))));
        }

        if mda_size < MIN_MDA_SIZE {
            return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                      format!("mda size {} is less than \
                                                               minimum ({})",
                                                              *mda_size,
                                                              *MIN_MDA_SIZE))));
        }

        let dev_info = try!(BlockDev::dev_info(paths, force));

        let mut bds = Vec::new();
        for (path, &(dev, dev_size)) in paths.iter().zip(dev_info.iter()) {

            let mut bd = BlockDev {
                pool_uuid: pool_uuid.to_owned(),
                dev_uuid: Uuid::new_v4(),
                dev: dev,
                path: path.to_path_buf(),
                sectors: Sectors(dev_size / SECTOR_SIZE),
                mdaa: MDA {
                    last_updated: Timespec::new(0, 0),
                    used: 0,
                    length: (*mda_size / 2 * SECTOR_SIZE) as u32,
                    crc: 0,
                    offset: SectorOffset(0),
                },
                mdab: MDA {
                    last_updated: Timespec::new(0, 0),
                    used: 0,
                    length: (*mda_size / 2 * SECTOR_SIZE) as u32,
                    crc: 0,
                    offset: SectorOffset(*mda_size / 2),
                },
                mda_sectors: mda_size,
                reserved_sectors: MDA_RESERVED_SIZE,
            };

            try!(bd.write_sigblock());
            bds.push(bd);
        }
        Ok(bds)
    }

    /// Gets device and device sizes, and returns an error if devices
    /// cannot be used by Stratis.
    fn dev_info(paths: &[&Path], force: bool) -> EngineResult<Vec<(Device, u64)>> {
        let mut dev_infos = Vec::new();
        for path in paths {
            let dev = try!(Device::from_str(&path.to_string_lossy()));

            let mut f = try!(OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|_| {
                    io::Error::new(ErrorKind::PermissionDenied,
                                   format!("Could not open {}", path.display()))
                }));

            if !force {
                let mut buf = [0u8; 4096];
                try!(f.read(&mut buf));

                if buf.iter().any(|x| *x != 0) {
                    return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                              format!("First 4K of {} is not \
                                                                       zeroed, and not forced",
                                                                      path.display()))));
                }
            }

            let dev_size = try!(blkdev_size(&f));
            if dev_size < MIN_DEV_SIZE {
                return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                          format!("{} too small, {} minimum",
                                                                  path.display(),
                                                                  ByteSize::b(MIN_DEV_SIZE as usize)
                                                                  .to_string(true)))));
            }
            dev_infos.push((dev, dev_size));
        }

        Ok(dev_infos)
    }

    pub fn setup(path: &Path) -> EngineResult<BlockDev> {
        let dev = try!(Device::from_str(&path.to_string_lossy()));

        let mut f = try!(OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|_| {
                io::Error::new(ErrorKind::PermissionDenied,
                               format!("Could not open {}", path.display()))
            }));

        let mut buf = [0u8; SECTOR_SIZE as usize];
        try!(f.seek(SeekFrom::Start(SECTOR_SIZE)));
        try!(f.read(&mut buf));

        if &buf[4..20] != STRAT_MAGIC {
            return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                      format!("{} is not a Stratis blockdev",
                                                              path.display()))));
        }

        let crc = crc32::checksum_ieee(&buf[4..SECTOR_SIZE as usize]);
        if crc != LittleEndian::read_u32(&buf[..4]) {
            dbgp!("{} header CRC failed", path.display());
            return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                      format!("{} header CRC failed",
                                                              path.display()))));
            // TODO: Try to read end-of-disk copy
        }

        let pool_id = try!(Uuid::parse_str(from_utf8(&buf[32..64]).unwrap())
            .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid pool uid")));
        let dev_id = try!(Uuid::parse_str(from_utf8(&buf[64..96]).unwrap())
            .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid dev uuid")));

        let mda_size = Sectors(LittleEndian::read_u32(&buf[160..164]) as u64);

        if *mda_size % 2 != 0 {
            return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                      format!("{} mda size is not an even \
                                                               number",
                                                              path.display()))));
        }

        if mda_size < MIN_MDA_SIZE {
            return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                      format!("{} mda size is less than \
                                                               minimum ({})",
                                                              path.display(),
                                                              *MIN_MDA_SIZE))));
        }

        Ok(BlockDev {
            pool_uuid: pool_id,
            dev_uuid: dev_id,
            dev: dev,
            path: path.to_owned(),
            sectors: Sectors(try!(blkdev_size(&f)) / SECTOR_SIZE),
            mdaa: MDA {
                last_updated: Timespec::new(LittleEndian::read_u64(&buf[64..72]) as i64,
                                            LittleEndian::read_u32(&buf[72..76]) as i32),
                used: LittleEndian::read_u32(&buf[76..80]),
                length: (*mda_size / 2 * SECTOR_SIZE) as u32,
                crc: LittleEndian::read_u32(&buf[80..84]),
                offset: SectorOffset(0),
            },
            mdab: MDA {
                last_updated: Timespec::new(LittleEndian::read_u64(&buf[96..104]) as i64,
                                            LittleEndian::read_u32(&buf[104..108]) as i32),
                used: LittleEndian::read_u32(&buf[108..112]),
                length: (*mda_size / 2 * SECTOR_SIZE) as u32,
                crc: LittleEndian::read_u32(&buf[112..116]),
                offset: SectorOffset(*mda_size / 2),
            },
            mda_sectors: mda_size,
            reserved_sectors: Sectors(LittleEndian::read_u32(&buf[164..168]) as u64),
        })
    }

    pub fn to_save(&self) -> BlockDevSave {
        BlockDevSave {
            path: self.path.clone(),
            sectors: self.sectors,
        }
    }

    pub fn find_all() -> EngineResult<Vec<BlockDev>> {
        Ok(try!(read_dir("/dev"))
            .into_iter()
            .filter_map(|dir_e| if dir_e.is_ok() {
                Some(dir_e.unwrap().path())
            } else {
                None
            })
            .filter_map(|path| BlockDev::setup(&path).ok())
            .collect::<Vec<_>>())
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

        let mut f = try!(OpenOptions::new().read(true).open(&self.path));
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

        let mut f = try!(OpenOptions::new().write(true).open(&self.path));

        // write metadata to disk
        try!(f.seek(SeekFrom::Start((*BDA_STATIC_HDR_SIZE + *older_mda.offset) * SECTOR_SIZE)));
        try!(f.write_all(&metadata));
        try!(f.seek(SeekFrom::End(-aux_bda_size)));
        try!(f.seek(SeekFrom::Current((*older_mda.offset * SECTOR_SIZE) as i64)));
        try!(f.write_all(&metadata));
        try!(f.flush());

        Ok(())
    }

    fn write_sigblock(&mut self) -> EngineResult<()> {
        let mut buf = [0u8; SECTOR_SIZE as usize];
        buf[4..20].clone_from_slice(STRAT_MAGIC);
        LittleEndian::write_u64(&mut buf[20..28], *self.sectors);
        // no flags yet
        buf[32..64].clone_from_slice(self.pool_uuid.simple().to_string().as_bytes());
        buf[64..96].clone_from_slice(self.dev_uuid.simple().to_string().as_bytes());

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

        try!(self.write_hdr_buf(&self.path, &buf));

        Ok(())
    }

    pub fn wipe_sigblock(&mut self) -> EngineResult<()> {
        let buf = [0u8; SECTOR_SIZE as usize];
        try!(self.write_hdr_buf(&self.path, &buf));
        Ok(())
    }

    fn write_hdr_buf(&self, path: &Path, buf: &[u8; SECTOR_SIZE as usize]) -> EngineResult<()> {
        let mut f = try!(OpenOptions::new().write(true).open(path));
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
        format!("{}:{}", self.dev.major, self.dev.minor)
    }
}
