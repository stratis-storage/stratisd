// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Actions that are shared between StratPool::teardown() and
/// StratPool::destroy(). It is necessary to make this a macro rather than
/// a function, because both methods consume their self argument.
macro_rules! teardown_pool {
    ( $s:ident ) => {
        let dm = try!(DM::new());
        for fs in $s.filesystems.empty() {
            try!(fs.teardown(&dm));
        }

        try!($s.thin_pool.teardown(&dm));
        try!($s.mdv.teardown(&dm));
    }
}
