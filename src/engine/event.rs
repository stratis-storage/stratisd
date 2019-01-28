// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fmt::Debug,
    sync::{Once, ONCE_INIT},
};

use crate::engine::{
    types::{BlockDevState, FreeSpaceState, PoolExtendState, PoolState},
    MaybeDbusPath,
};

static INIT: Once = ONCE_INIT;
static mut ENGINE_LISTENER_LIST: Option<EngineListenerList> = None;

#[derive(Debug, Clone)]
pub enum EngineEvent<'a> {
    BlockdevStateChanged {
        dbus_path: &'a MaybeDbusPath,
        state: BlockDevState,
    },
    FilesystemRenamed {
        dbus_path: &'a MaybeDbusPath,
        from: &'a str,
        to: &'a str,
    },
    PoolExtendStateChanged {
        dbus_path: &'a MaybeDbusPath,
        state: PoolExtendState,
    },
    PoolRenamed {
        dbus_path: &'a MaybeDbusPath,
        from: &'a str,
        to: &'a str,
    },
    PoolSpaceStateChanged {
        dbus_path: &'a MaybeDbusPath,
        state: FreeSpaceState,
    },
    PoolStateChanged {
        dbus_path: &'a MaybeDbusPath,
        state: PoolState,
    },
}

pub trait EngineListener: Debug {
    fn notify(&self, event: &EngineEvent);
}

#[derive(Debug)]
pub struct EngineListenerList {
    listeners: Vec<Box<dyn EngineListener>>,
}

impl EngineListenerList {
    /// Create a new EngineListenerList.
    pub fn new() -> EngineListenerList {
        EngineListenerList {
            listeners: Vec::new(),
        }
    }

    /// Add a listener.
    pub fn register_listener(&mut self, listener: Box<dyn EngineListener>) {
        self.listeners.push(listener);
    }

    /// Notify a listener.
    pub fn notify(&self, event: &EngineEvent) {
        for listener in &self.listeners {
            listener.notify(&event);
        }
    }
}

impl Default for EngineListenerList {
    fn default() -> EngineListenerList {
        EngineListenerList::new()
    }
}

pub fn get_engine_listener_list() -> &'static EngineListenerList {
    unsafe {
        INIT.call_once(|| ENGINE_LISTENER_LIST = Some(EngineListenerList::new()));
        match ENGINE_LISTENER_LIST {
            Some(ref mut ell) => ell,
            _ => panic!("ENGINE_LISTENER_LIST is None"),
        }
    }
}

pub fn get_engine_listener_list_mut() -> &'static mut EngineListenerList {
    unsafe {
        INIT.call_once(|| ENGINE_LISTENER_LIST = Some(EngineListenerList::new()));
        match ENGINE_LISTENER_LIST {
            Some(ref mut ell) => ell,
            _ => panic!("ENGINE_LISTENER_LIST is None"),
        }
    }
}
