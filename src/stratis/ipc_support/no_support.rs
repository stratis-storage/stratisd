// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub struct IpcSupport;

// If IPC is compiled out, do very little.
impl IpcSupport {
    pub fn setup(_engine: &Rc<RefCell<dyn Engine>>) -> StratisResult<IpcSupport> {
        Ok(IpcSupport)
    }

    pub fn process(&mut self, _fds: &mut Vec<libc::pollfd>, _dbus_client_index_start: usize) {}

    pub fn register_pool(&mut self, _pool_name: &Name, _pool_uuid: PoolUuid, _pool: &mut dyn Pool) {
    }
}
