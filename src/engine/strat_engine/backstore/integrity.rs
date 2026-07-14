// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::{hash_map::Entry, HashMap};

use devicemapper::{
    Device, LinearDev, LinearDevTargetParams, LinearTargetParams, Sectors, TargetLine,
};

use crate::{
    engine::{
        strat_engine::{
            backstore::{blockdev::v2::StratBlockDev, data_tier::DataTier},
            dm::get_dm,
            names::{format_integrity_ids, IntegrityRole},
        },
        types::PoolUuid,
    },
    stratis::{StratisError, StratisResult},
};

type IntegritySegments = (
    HashMap<Device, Vec<(Sectors, Sectors)>>,
    HashMap<Device, Vec<(Sectors, Sectors)>>,
);

/// Generate a better format for the integrity data and metadata segments when setting up the
/// linear devices.
#[allow(dead_code)]
fn generate_segments_from_metadata(
    data_tier: DataTier<StratBlockDev>,
) -> StratisResult<IntegritySegments> {
    let data_segments = data_tier
        .segments
        .inner
        .iter()
        .try_fold::<_, _, StratisResult<_>>(
            HashMap::new(),
            |mut hash: HashMap<_, Vec<(Sectors, Sectors)>>, seg| {
                let (_, blockdev) = data_tier.get_blockdev_by_uuid(seg.uuid).ok_or_else(|| {
                    StratisError::Msg(format!(
                        "No record of device with UUID {} found in active block device manager",
                        seg.uuid
                    ))
                })?;
                match hash.entry(*blockdev.device()) {
                    Entry::Occupied(mut entry) => {
                        entry
                            .get_mut()
                            .push((seg.segment.start, seg.segment.length));
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(vec![(seg.segment.start, seg.segment.length)]);
                    }
                };
                Ok(hash)
            },
        )?;

    let meta_segments = data_tier.blockdevs().into_iter().fold(
        HashMap::new(),
        |mut hash: HashMap<_, Vec<(Sectors, Sectors)>>, (_, bd)| {
            match hash.entry(*bd.device()) {
                Entry::Occupied(mut entry) => entry.get_mut().extend(bd.meta_allocs()),
                Entry::Vacant(entry) => {
                    entry.insert(bd.meta_allocs());
                }
            };
            hash
        },
    );

    Ok((data_segments, meta_segments))
}

/// Set up a linear device to be used as an integrity subdevice from a record of allocations in the
/// metadata.
fn setup_linear_dev(
    pool_uuid: PoolUuid,
    devno: Device,
    role: IntegrityRole,
    info: &HashMap<Device, Vec<(Sectors, Sectors)>>,
) -> StratisResult<LinearDev> {
    let (name, uuid) = format_integrity_ids(pool_uuid, role);
    let (_, linear_table) = info
        .get(&devno)
        .ok_or_else(|| {
            StratisError::Msg(format!(
                "Failed to find a record of allocations for device number {devno}"
            ))
        })?
        .iter()
        .fold(
            (Sectors(0), Vec::new()),
            |(mut offset, mut vec), (start, length)| {
                vec.push(TargetLine::new(
                    offset,
                    *length,
                    LinearDevTargetParams::Linear(LinearTargetParams::new(devno, *start)),
                ));
                offset += *length;
                (offset, vec)
            },
        );
    LinearDev::setup(get_dm(), &name, Some(&uuid), linear_table).map_err(StratisError::from)
}

/// Represents a handle to the integrity layer of the devicemapper stack.
#[allow(dead_code)]
pub struct IntegrityLayer {
    data: LinearDev,
    meta: LinearDev,
}

impl IntegrityLayer {
    /// Initialize the integrity layer for a pool.
    #[allow(dead_code)]
    fn initialize(
        pool_uuid: PoolUuid,
        devno: Device,
        data_segments: &HashMap<Device, Vec<(Sectors, Sectors)>>,
        metadata_segments: &HashMap<Device, Vec<(Sectors, Sectors)>>,
    ) -> StratisResult<Self> {
        let data_dev = setup_linear_dev(pool_uuid, devno, IntegrityRole::OriginSub, data_segments)?;

        let meta_dev =
            setup_linear_dev(pool_uuid, devno, IntegrityRole::MetaSub, metadata_segments)?;

        Ok(IntegrityLayer {
            data: data_dev,
            meta: meta_dev,
        })
    }
}
