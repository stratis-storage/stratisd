// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use libc::c_uint;

use devicemapper::{Bytes, Sectors, IEC};

// Stratis token JSON keys
pub const TOKEN_TYPE_KEY: &str = "type";
pub const TOKEN_KEYSLOTS_KEY: &str = "keyslots";
pub const STRATIS_TOKEN_DEVNAME_KEY: &str = "activation_name";
pub const STRATIS_TOKEN_POOL_UUID_KEY: &str = "pool_uuid";
pub const STRATIS_TOKEN_DEV_UUID_KEY: &str = "device_uuid";
pub const STRATIS_TOKEN_POOLNAME_KEY: &str = "pool_name";

pub const STRATIS_TOKEN_ID: c_uint = 0;
pub const LUKS2_TOKEN_ID: c_uint = 1;
pub const CLEVIS_LUKS_TOKEN_ID: c_uint = 2;

pub const LUKS2_TOKEN_TYPE: &str = "luks2-keyring";
pub const STRATIS_TOKEN_TYPE: &str = "stratis";

/// The size of the media encryption key generated by cryptsetup for
/// each block device.
pub const STRATIS_MEK_SIZE: usize = 512 / 8;

/// Sector size as defined in the LUKS2 specification documentation.
pub const LUKS2_SECTOR_SIZE: Bytes = Bytes(4096);

/// Key in clevis configuration for tang indicating that the URL of the
/// tang server does not need to be verified.
pub const CLEVIS_TANG_TRUST_URL: &str = "stratis:tang:trust_url";

pub const DEFAULT_CRYPT_METADATA_SIZE_V1: Bytes = Bytes(16 * IEC::Ki as u128);
pub const DEFAULT_CRYPT_METADATA_SIZE_V2: Bytes = Bytes(64 * IEC::Ki as u128);
pub const DEFAULT_CRYPT_KEYSLOTS_SIZE: Bytes = Bytes(16352 * IEC::Ki as u128);
pub const DEFAULT_CRYPT_DATA_OFFSET_V2: Sectors = Sectors(34816);

pub const CLEVIS_TOKEN_NAME: &str = "clevis\0";

pub const CLEVIS_RECURSION_LIMIT: u64 = 20;
