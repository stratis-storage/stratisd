// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! A logger that wraps another logger, and provides "flight data recorder"
//! semantics: nothing is output until asked for. This can be by calling
//! `Handle::dump()`, or a `HandleGuard` can be used to dump the log when a
//! scope is exited.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Duration, Utc};
use log::{self, Level, Log, Metadata, MetadataBuilder, Record};

const LOCK_EXPECT_MSG: &str =
    "No code in this module can panic; therefore the mutex can not be poisoned.";

#[derive(Debug, Clone)]
/// A structure that allows interaction with the installed buff_log.
pub struct Handle<L: Log> {
    shared: Arc<Mutex<BuffLogger<L>>>,
}

impl<L: Log> Handle<L> {
    fn new(shared: &Arc<Mutex<BuffLogger<L>>>) -> Handle<L> {
        Handle {
            shared: shared.clone(),
        }
    }

    /// Send buffered logs to the wrapped logger.
    pub fn dump(&self) {
        let shared = self.shared.lock().expect(LOCK_EXPECT_MSG);
        let mut vec = shared.buff.lock().expect(LOCK_EXPECT_MSG);
        for (time, item) in vec.drain(..) {
            shared.log.log(
                &Record::builder()
                    .metadata(
                        MetadataBuilder::new()
                            .level(item.metadata.level)
                            .target(&item.metadata.target)
                            .build(),
                    )
                    .args(format_args!("{} {}", time, item.args))
                    .file(item.file.as_deref())
                    .line(item.line)
                    .module_path(item.module_path.as_deref())
                    .build(),
            );
        }
    }

    pub fn buffered_count(&self) -> usize {
        let shared = self.shared.lock().expect(LOCK_EXPECT_MSG);
        let vec = shared.buff.lock().expect(LOCK_EXPECT_MSG);
        vec.len()
    }

    /// Construct a HandleGuard, that will call `dump()` when it goes out of
    /// scope.
    pub fn to_guard(&self) -> HandleGuard<L> {
        HandleGuard {
            handle: Handle::new(&self.shared.clone()),
        }
    }
}

/// A structure that will output all buffered log lines when it leaves scope.
pub struct HandleGuard<L: Log> {
    handle: Handle<L>,
}

impl<L: Log> Drop for HandleGuard<L> {
    fn drop(&mut self) {
        self.handle.dump()
    }
}

/// Create a new BuffLog wrapping another implementer of the `Log` trait.
#[derive(Debug)]
pub struct Logger<L: Log>(Arc<Mutex<BuffLogger<L>>>);

impl<L: Log + 'static> Logger<L> {
    /// If `pass_through` is `true`, no buffering is performed. One may wish to disable
    /// buffering in some cases, and this is an easy way to do it.
    /// If `hold_time` is given, log messages may be discarded after this time passes.
    /// `None` means keep indefinitely.
    pub fn new(logger: L, pass_through: bool, hold_time: Option<Duration>) -> Logger<L> {
        Logger(Arc::new(Mutex::new(BuffLogger::new(
            logger,
            pass_through,
            hold_time,
        ))))
    }

    /// Set buff_log as the global logging instance.
    pub fn init(self) -> Handle<L> {
        let (pass_through, hold_time) = {
            let shared = self.0.lock().expect(LOCK_EXPECT_MSG);
            (shared.pass_through, shared.hold_time)
        };
        let handle = Handle::new(&self.0);
        log::set_max_level(Level::max().to_level_filter());
        log::set_boxed_logger(Box::new(self)).expect("set_logger should only be called once");
        debug!(
            "BuffLogger: pass_through: {} hold time: {}",
            pass_through,
            hold_time
                .map(|d| d.to_string())
                .unwrap_or_else(|| "none".into())
        );
        handle
    }
}

impl<L: Log> Log for Logger<L> {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.0.lock().expect(LOCK_EXPECT_MSG).log.enabled(metadata)
    }
    fn log(&self, record: &Record) {
        let shared = self.0.lock().expect(LOCK_EXPECT_MSG);
        if shared.pass_through {
            shared.log.log(record)
        } else {
            let now = Utc::now();
            let mut v = shared.buff.lock().expect(LOCK_EXPECT_MSG);
            v.push_back((now, OwnedRecord::from_record(record)));

            // Drop entries that are older than hold time
            if let Some(hold_time) = shared.hold_time {
                v.retain(|&(time, _)| time + hold_time >= now);
            }
        }
    }
    fn flush(&self) {
        self.0.lock().expect(LOCK_EXPECT_MSG).log.flush()
    }
}

/// An owned version of Metadata
#[derive(Debug)]
struct OwnedMetadata {
    level: Level,
    target: String,
}

/// An owned version of `log::Record`
#[derive(Debug)]
struct OwnedRecord {
    metadata: OwnedMetadata,
    args: String,
    module_path: Option<String>,
    file: Option<String>,
    line: Option<u32>,
}

impl OwnedRecord {
    fn from_record(record: &Record) -> OwnedRecord {
        OwnedRecord {
            metadata: OwnedMetadata {
                level: record.metadata().level(),
                target: record.metadata().target().into(),
            },
            args: format!("{}", record.args()),
            module_path: record.module_path().map(|s| s.to_owned()),
            file: record.file().map(|s| s.to_owned()),
            line: record.line(),
        }
    }
}

#[allow(clippy::type_complexity)]
#[derive(Debug)]
struct BuffLogger<L: Log> {
    pass_through: bool,
    hold_time: Option<Duration>,
    log: L,
    buff: Arc<Mutex<VecDeque<(DateTime<Utc>, OwnedRecord)>>>,
}

impl<L: Log> BuffLogger<L> {
    fn new(log: L, pass_through: bool, hold_time: Option<Duration>) -> BuffLogger<L> {
        BuffLogger {
            pass_through,
            hold_time,
            log,
            buff: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestLog {}

    impl Log for TestLog {
        fn enabled(&self, _metadata: &Metadata) -> bool {
            true
        }
        fn log(&self, record: &Record) {
            println!(
                "{}:{} -- {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
        fn flush(&self) {}
    }

    #[test]
    fn test_buffering() {
        let logger = Logger::new(TestLog {}, false, None);
        let handle = logger.init();

        // buff-log adds its own initial (buffered) log message
        assert_eq!(handle.buffered_count(), 1);

        error!("error 1");
        warn!("warn 1");
        info!("info 1");
        debug!("debug 1");
        trace!("trace 1");

        assert_eq!(handle.buffered_count(), 6);
        handle.dump();
        assert_eq!(handle.buffered_count(), 0);
    }
}
