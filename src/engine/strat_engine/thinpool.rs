// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Code to handle management of a pool's thinpool device.

use std::process::Command;

use devicemapper;
use devicemapper::{Bytes, DM, DataBlocks, DmError, DmResult, LinearDev, MetaBlocks, Sectors,
                   Segment, ThinDev, ThinDevId, ThinPoolDev, ThinPoolStatus};

use super::super::consts::IEC;
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::types::PoolUuid;

use super::dmdevice::{FlexRole, ThinDevIdPool, ThinPoolRole, format_flex_name,
                      format_thinpool_name};
use super::serde_structs::{Recordable, ThinPoolDevSave};

pub const DATA_BLOCK_SIZE: Sectors = Sectors(2048);
pub const DATA_LOWATER: DataBlocks = DataBlocks(512);
pub const META_LOWATER: MetaBlocks = MetaBlocks(512);

/// A ThinPool struct contains the thinpool itself, but also the spare
/// segments for its metadata device.
#[derive(Debug)]
pub struct ThinPool {
    thin_pool: ThinPoolDev,
    meta_spare: Vec<Segment>,
    id_gen: ThinDevIdPool,
}

impl ThinPool {
    /// Make a new thin pool.
    pub fn new(pool_uuid: PoolUuid,
               dm: &DM,
               data_block_size: Sectors,
               low_water_mark: DataBlocks,
               spare_segments: Vec<Segment>,
               meta_dev: LinearDev,
               data_dev: LinearDev)
               -> EngineResult<ThinPool> {
        let name = format_thinpool_name(&pool_uuid, ThinPoolRole::Pool);
        let thinpool_dev = try!(ThinPoolDev::new(&name,
                                                 dm,
                                                 try!(data_dev.size()),
                                                 data_block_size,
                                                 low_water_mark,
                                                 meta_dev,
                                                 data_dev));
        Ok(ThinPool {
               thin_pool: thinpool_dev,
               meta_spare: spare_segments,
               id_gen: ThinDevIdPool::new_from_ids(&[]),
           })
    }

    /// Set up an "existing" thin pool.
    /// A thin pool must store the metadata for its thin devices, regardless of
    /// whether it has an existing device node. An existing thin pool device
    /// is a device where the metadata is already stored on its meta device.
    /// If initial setup fails due to a thin_check failure, attempt to fix
    /// the problem by running thin_repair. If failure recurs, return an
    /// error.
    pub fn setup(pool_uuid: PoolUuid,
                 dm: &DM,
                 data_block_size: Sectors,
                 low_water_mark: DataBlocks,
                 thin_ids: &[ThinDevId],
                 spare_segments: Vec<Segment>,
                 meta_dev: LinearDev,
                 data_dev: LinearDev)
                 -> EngineResult<ThinPool> {
        let name = format_thinpool_name(&pool_uuid, ThinPoolRole::Pool);
        let size = try!(data_dev.size());
        match ThinPoolDev::setup(&name,
                                 dm,
                                 size,
                                 data_block_size,
                                 low_water_mark,
                                 meta_dev,
                                 data_dev) {
            Ok(dev) => {
                Ok(ThinPool {
                       thin_pool: dev,
                       meta_spare: spare_segments,
                       id_gen: ThinDevIdPool::new_from_ids(thin_ids),
                   })
            }
            Err(DmError::Dm(devicemapper::ErrorEnum::CheckFailed(meta_dev, data_dev), _)) => {
                let (new_meta_dev, new_spare_segments) =
                    try!(attempt_thin_repair(pool_uuid, dm, meta_dev, spare_segments));
                Ok(ThinPool {
                       thin_pool: try!(ThinPoolDev::setup(&name,
                                                          dm,
                                                          size,
                                                          data_block_size,
                                                          low_water_mark,
                                                          new_meta_dev,
                                                          data_dev)),
                       meta_spare: new_spare_segments,
                       id_gen: ThinDevIdPool::new_from_ids(thin_ids),
                   })
            }
            Err(err) => Err(err.into()),
        }
    }

    /// The status of the thin pool as calculated by DM.
    pub fn thin_pool_status(&self, dm: &DM) -> EngineResult<ThinPoolStatus> {
        Ok(try!(self.thin_pool.status(dm)))
    }

    /// Make a new thin device.
    pub fn make_thin_device(&mut self,
                            dm: &DM,
                            name: &str,
                            size: Option<Sectors>)
                            -> EngineResult<ThinDev> {
        Ok(try!(ThinDev::new(name,
                             dm,
                             &self.thin_pool,
                             try!(self.id_gen.new_id()),
                             size.unwrap_or(Bytes(IEC::Ti).sectors()))))
    }

    /// Setup a previously constructed thin device.
    pub fn setup_thin_device(&self,
                             dm: &DM,
                             name: &str,
                             id: ThinDevId,
                             size: Sectors)
                             -> EngineResult<ThinDev> {
        Ok(try!(ThinDev::setup(name, dm, &self.thin_pool, id, size)))
    }

    /// Tear down the thin pool.
    pub fn teardown(self, dm: &DM) -> DmResult<()> {
        self.thin_pool.teardown(dm)
    }

    /// Get an immutable reference to the sparse segments of the ThinPool.
    pub fn spare_segments(&self) -> &[Segment] {
        &self.meta_spare
    }

    /// The segments belonging to the thin pool meta device.
    pub fn thin_pool_meta_segments(&self) -> &[Segment] {
        self.thin_pool.meta_dev().segments()
    }

    /// The segments belonging to the thin pool data device.
    pub fn thin_pool_data_segments(&self) -> &[Segment] {
        self.thin_pool.data_dev().segments()
    }

    /// Extend the thinpool with new data regions.
    pub fn extend_data(&mut self, dm: &DM, segs: Vec<Segment>) -> EngineResult<()> {
        Ok(try!(self.thin_pool.extend_data(dm, segs)))
    }

    /// The number of physical sectors in use, that is, unavailable for storage
    /// of additional user data, by this pool.
    // This includes all the sectors being held as spares for the meta device,
    // all the sectors allocated to the meta data device, and all the sectors
    // in use on the data device.
    pub fn total_physical_used(&self) -> EngineResult<Sectors> {
        let data_dev_used = match try!(self.thin_pool.status(&try!(DM::new()))) {
            ThinPoolStatus::Good(_, usage) => *usage.used_data * DATA_BLOCK_SIZE,
            _ => {
                let err_msg = "thin pool failed, could not obtain usage";
                return Err(EngineError::Engine(ErrorEnum::Invalid, err_msg.into()));
            }
        };

        let spare_total = self.spare_segments().iter().map(|s| s.length).sum();
        let meta_dev_total = self.thin_pool
            .meta_dev()
            .segments()
            .iter()
            .map(|s| s.length)
            .sum();

        Ok(data_dev_used + spare_total + meta_dev_total)
    }
}

impl Recordable<ThinPoolDevSave> for ThinPool {
    fn record(&self) -> EngineResult<ThinPoolDevSave> {
        Ok(ThinPoolDevSave { data_block_size: self.thin_pool.data_block_size() })
    }
}

/// Attempt a thin repair operation on the meta device.
/// If the operation succeeds, teardown the old meta device,
/// and return the new meta device and the new spare segments.
fn attempt_thin_repair(pool_uuid: PoolUuid,
                       dm: &DM,
                       meta_dev: LinearDev,
                       mut spare_segments: Vec<Segment>)
                       -> EngineResult<(LinearDev, Vec<Segment>)> {
    let mut new_meta_dev = try!(LinearDev::new(&format_flex_name(&pool_uuid,
                                                                 FlexRole::ThinMetaSpare),
                                               dm,
                                               spare_segments.drain(..).collect()));


    if try!(Command::new("thin_repair")
                .arg("-i")
                .arg(&try!(meta_dev.devnode()))
                .arg("-o")
                .arg(&try!(new_meta_dev.devnode()))
                .status())
               .success() == false {
        return Err(EngineError::Engine(ErrorEnum::Error,
                                       "thin_repair failed, pool unusable".into()));
    }

    let name = meta_dev.name().to_owned();
    let new_spare_segments = meta_dev
        .segments()
        .iter()
        .map(|x| {
                 Segment {
                     start: x.start,
                     length: x.length,
                     device: x.device,
                 }
             })
        .collect();
    try!(meta_dev.teardown(dm));
    try!(new_meta_dev.set_name(dm, &name));

    Ok((new_meta_dev, new_spare_segments))
}
