// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.


use consts::*;
use types::Sectors;

pub const STRAT_MAGIC: &'static [u8] = b"!Stra0tis\x86\xff\x02^\x41rh";

pub const MIN_DEV_SIZE: u64 = GIGA;

pub const BDA_STATIC_HDR_SIZE: Sectors = Sectors(8);
pub const MIN_MDA_SIZE: Sectors = Sectors(2040);

pub const MDA_RESERVED_SIZE: Sectors = Sectors(2048 * 3); // 3 MiB
