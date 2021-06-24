// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{error::Error, fmt, io, str, sync};

pub type StratisResult<T> = Result<T, StratisError>;

#[derive(Debug)]
pub enum StratisError {
    Msg(String),
    Chained(String, Box<StratisError>),
    /// This variant is for rollback operations like wiping block devices
    /// where the operation may continue even if there are failures
    /// as there is no action to be taken in the event of failures. We
    /// just need to report all of the failures to the users.
    BestEffortError(String, Vec<StratisError>),
    RollbackError {
        causal_error: Box<StratisError>,
        rollback_error: Box<StratisError>,
    },
    /// This variant should be used for failed roll back that does not
    /// prompt any action in stratisd but needs to be reported to the user.
    NoActionRollbackError {
        causal_error: Box<StratisError>,
        rollback_error: Box<StratisError>,
    },
    Io(io::Error),
    Nix(nix::Error),
    Uuid(uuid::Error),
    Utf8(str::Utf8Error),
    Serde(serde_json::error::Error),
    Decode(data_encoding::DecodeError),
    DM(devicemapper::DmError),
    Crypt(libcryptsetup_rs::LibcryptErr),
    Recv(sync::mpsc::RecvError),
    Null(std::ffi::NulError),
    Join(tokio::task::JoinError),

    #[cfg(feature = "dbus_enabled")]
    Dbus(dbus::Error),
    Udev(libudev::Error),
}

impl fmt::Display for StratisError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            StratisError::Msg(ref s) => write!(f, "{}", s),
            StratisError::Chained(ref s, ref chained) => write!(f, "{}; {}", s, chained),
            StratisError::BestEffortError(ref s, ref errs) => {
                if errs.is_empty() {
                    write!(f, "{}", s)
                } else {
                    let errs_string = errs
                        .iter()
                        .map(|err| err.to_string())
                        .collect::<Vec<_>>()
                        .join("; ");
                    write!(f, "{}; {}", s, errs_string)
                }
            }
            StratisError::RollbackError {
                ref causal_error,
                ref rollback_error,
            } => {
                write!(f, "Rollback failed; causal_error: {}, rollback error: {}; putting pool in maintenance mode", causal_error, rollback_error)
            }
            StratisError::NoActionRollbackError {
                ref causal_error,
                ref rollback_error,
            } => {
                write!(
                    f,
                    "Rollback failed; causal_error: {}, rollback error: {}",
                    causal_error, rollback_error
                )
            }
            StratisError::Io(ref err) => write!(f, "IO error: {}", err),
            StratisError::Nix(ref err) => write!(f, "Nix error: {}", err),
            StratisError::Uuid(ref err) => write!(f, "Uuid error: {}", err),
            StratisError::Utf8(ref err) => write!(f, "Utf8 error: {}", err),
            StratisError::Serde(ref err) => write!(f, "Serde error: {}", err),
            StratisError::Decode(ref err) => write!(f, "Data encoding error: {}", err),
            StratisError::DM(ref err) => write!(f, "DM error: {}", err),
            StratisError::Crypt(ref err) => write!(f, "Cryptsetup error: {}", err),
            StratisError::Recv(ref err) => write!(f, "Synchronization channel error: {}", err),
            StratisError::Null(ref err) => write!(f, "C string conversion error: {}", err),
            StratisError::Join(ref err) => write!(f, "Failed to join thread: {}", err),

            #[cfg(feature = "dbus_enabled")]
            StratisError::Dbus(ref err) => {
                write!(f, "Dbus error: {}", err.message().unwrap_or("Unknown"))
            }
            StratisError::Udev(ref err) => write!(f, "Udev error: {}", err),
        }
    }
}

impl Error for StratisError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match *self {
            StratisError::Msg(_) => None,
            StratisError::Chained(_, ref chained) => Some(chained),
            StratisError::RollbackError {
                ref causal_error, ..
            } => Some(causal_error),
            StratisError::NoActionRollbackError {
                ref causal_error, ..
            } => Some(causal_error),
            StratisError::BestEffortError(_, _) => None,
            StratisError::Io(ref err) => Some(err),
            StratisError::Nix(ref err) => Some(err),
            StratisError::Uuid(ref err) => Some(err),
            StratisError::Utf8(ref err) => Some(err),
            StratisError::Serde(ref err) => Some(err),
            StratisError::Decode(ref err) => Some(err),
            StratisError::DM(ref err) => Some(err),
            StratisError::Crypt(ref err) => Some(err),
            StratisError::Recv(ref err) => Some(err),
            StratisError::Null(ref err) => Some(err),
            StratisError::Join(ref err) => Some(err),

            #[cfg(feature = "dbus_enabled")]
            StratisError::Dbus(ref err) => Some(err),
            StratisError::Udev(ref err) => Some(err),
        }
    }
}

impl From<tokio::task::JoinError> for StratisError {
    fn from(err: tokio::task::JoinError) -> StratisError {
        StratisError::Join(err)
    }
}

impl From<std::ffi::NulError> for StratisError {
    fn from(err: std::ffi::NulError) -> StratisError {
        StratisError::Null(err)
    }
}

impl From<io::Error> for StratisError {
    fn from(err: io::Error) -> StratisError {
        StratisError::Io(err)
    }
}

impl From<nix::Error> for StratisError {
    fn from(err: nix::Error) -> StratisError {
        StratisError::Nix(err)
    }
}

impl From<uuid::Error> for StratisError {
    fn from(err: uuid::Error) -> StratisError {
        StratisError::Uuid(err)
    }
}

impl From<str::Utf8Error> for StratisError {
    fn from(err: str::Utf8Error) -> StratisError {
        StratisError::Utf8(err)
    }
}

impl From<serde_json::error::Error> for StratisError {
    fn from(err: serde_json::error::Error) -> StratisError {
        StratisError::Serde(err)
    }
}

impl From<data_encoding::DecodeError> for StratisError {
    fn from(err: data_encoding::DecodeError) -> StratisError {
        StratisError::Decode(err)
    }
}

impl From<devicemapper::DmError> for StratisError {
    fn from(err: devicemapper::DmError) -> StratisError {
        StratisError::DM(err)
    }
}

impl From<libcryptsetup_rs::LibcryptErr> for StratisError {
    fn from(err: libcryptsetup_rs::LibcryptErr) -> StratisError {
        StratisError::Crypt(err)
    }
}

#[cfg(feature = "dbus_enabled")]
impl From<dbus::Error> for StratisError {
    fn from(err: dbus::Error) -> StratisError {
        StratisError::Dbus(err)
    }
}

impl From<libudev::Error> for StratisError {
    fn from(err: libudev::Error) -> StratisError {
        StratisError::Udev(err)
    }
}

impl From<sync::mpsc::RecvError> for StratisError {
    fn from(err: sync::mpsc::RecvError) -> StratisError {
        StratisError::Recv(err)
    }
}
