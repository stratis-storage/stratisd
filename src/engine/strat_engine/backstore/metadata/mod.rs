// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod bda;
mod mda;
mod static_header;

pub use self::{bda::BDA, mda::MDADataSize, static_header::StaticHeader};
