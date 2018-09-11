// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fmt::Debug;
use std::sync::{Once, ONCE_INIT};

use super::types::{BlockDevState, MaybeDbusPath};

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
    PoolRenamed {
        dbus_path: &'a MaybeDbusPath,
        from: &'a str,
        to: &'a str,
    },
}

pub trait EngineListener: Debug {
    fn notify(&self, event: &EngineEvent);
}

#[derive(Debug)]
pub struct EngineListenerList {
    listeners: Vec<Box<EngineListener>>,
}

impl EngineListenerList {
    /// Create a new EngineListenerList.
    pub fn new() -> EngineListenerList {
        EngineListenerList {
            listeners: Vec::new(),
        }
    }

    /// Add a listener.
    // This code is marked dead because it is called only by bin/stratisd.rs
    #[allow(dead_code)]
    pub fn register_listener(&mut self, listener: Box<EngineListener>) {
        self.listeners.push(listener);
    }

    /// Notify a listener.
    pub fn notify(&self, event: &EngineEvent) {
        for listener in self.listeners.iter() {
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
