// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(feature = "dbus_enabled")]
use dbus;

use std::fmt::Debug;

#[derive(Debug, Clone)]
pub enum EngineEvent<'a> {
    PoolRenamed {
        #[cfg(feature = "dbus_enabled")]
        dbus_path: &'a Option<dbus::Path<'static>>,
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
    pub fn register_listener(&mut self, listener: Box<EngineListener>) {
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
