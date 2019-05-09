// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Defines an error type to be used by the stratis engine.

use std::process::Output;

use backtrace::Backtrace;

#[derive(Debug)]
/// The ErrorKind differentiates among different errors.
pub enum ErrorKind {
    /// Binaries that stratisd relies on for operation not available.
    /// names is the names of all binaries not found.
    /// locations lists the locations searched.
    BinariesNotFound {
        names: Vec<String>,
        locations: Vec<String>,
    },

    /// The attempt to execute an external binary failed
    /// cmd is a string representation of the command.
    CommandExecutionFailure { cmd: String },

    /// An external binary was executed but it returned an error code.
    CommandFailure { cmd: String, output: Output },

    /// The name specified for a stratis entity is invalid.
    // FIXME: the reason should be its own enum.
    InvalidName { name: String, reason: String },

    /// The checksum calculated for the MDA Header does not agree with the
    /// expected value.
    MDAHeaderChecksumIncorrect { expected: u32, actual: u32 },
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ErrorKind::BinariesNotFound { names, locations } => write!(
                f,
                "executables not found: [{}], locations searched: [{}]",
                names.join(" ,"),
                locations.join(" ,")
            ),
            ErrorKind::CommandExecutionFailure { cmd } => {
                write!(f, "failed to execute cmd {}", cmd)
            }
            ErrorKind::CommandFailure { cmd, output } => write!(
                f,
                "command {} failed. status: {}, stdout: \"{}\", stderr:\"{}\"",
                cmd,
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            ErrorKind::InvalidName { name, reason } => write!(
                f,
                "invalid name \"{}\" for a Stratis entity, reason: {}",
                name, reason
            ),
            ErrorKind::MDAHeaderChecksumIncorrect { expected, actual } => write!(
                f,
                "expected checksum for MDAHeader {} not equal to actual checksum {}",
                expected, actual
            ),
        }
    }
}

#[derive(Debug)]
/// What relation the component error has to its parent
enum Suberror {
    /// The error occurred before the parent error
    Previous(Box<(dyn std::error::Error + Send)>),
    /// The error is further explained or extended by the parent
    Constituent(Box<(dyn std::error::Error + Send)>),
}

#[derive(Debug)]
pub struct Error {
    // The source of the error, which may be an error for
    // which this error is a further explanation, i.e., a
    // constituent error, or it may simply be an error that occurred
    // previously, and which presumably caused the current code to
    // be run and encounter its own, novel error.
    source_impl: Option<Suberror>,

    // The backtrace at the site the error is returned
    backtrace: Backtrace,

    // Distinguish among different errors with an ErrorKind
    pub specifics: ErrorKind,
}

impl Error {
    pub fn new(kind: ErrorKind) -> Error {
        Error {
            backtrace: Backtrace::new(),
            source_impl: None,
            specifics: kind,
        }
    }

    /// Return the optional backtrace associated with this error.
    // Note that the function name is our_backtrace, so that it does not
    // conflict with a future possible backtrace function in the Error trait.
    pub fn our_backtrace(&self) -> Option<&Backtrace> {
        Some(&self.backtrace)
    }

    /// Set extension as the extension on this error.
    /// Return the head of the chain, now subsequent.
    pub fn set_extension(self, mut extension: Error) -> Error {
        extension.source_impl = Some(Suberror::Constituent(Box::new(self)));
        extension
    }

    /// Set subsequent as the subsequent error for this error.
    /// Return the head of the chain, now subsequent.
    pub fn set_subsequent(self, mut subsequent: Error) -> Error {
        subsequent.source_impl = Some(Suberror::Previous(Box::new(self)));
        subsequent
    }

    /// Set constituent as the constituent of this error.
    pub fn set_constituent(mut self, constituent: Box<dyn std::error::Error + Send>) -> Error {
        self.source_impl = Some(Suberror::Constituent(constituent));
        self
    }

    /// Set previous as the previous error.
    pub fn set_previous(mut self, previous: Box<dyn std::error::Error + Send>) -> Error {
        self.source_impl = Some(Suberror::Previous(previous));
        self
    }

    /// Obtain the immediate previous error, if there is one
    pub fn previous(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self.source_impl.as_ref() {
            Some(Suberror::Previous(c)) => Some(&**c),
            _ => None,
        }
    }

    /// Obtain the immediate constituent error, if there is one
    pub fn constituent(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self.source_impl.as_ref() {
            Some(Suberror::Constituent(c)) => Some(&**c),
            _ => None,
        }
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Error {
        Error::new(kind)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source_impl.as_ref().map(|c| match c {
            Suberror::Previous(c) => &**c as &(dyn std::error::Error + 'static),
            Suberror::Constituent(c) => &**c as &(dyn std::error::Error + 'static),
        })
    }

    // deprecated in 1.33.0
    // identical to source()
    fn cause(&self) -> Option<&dyn std::error::Error> {
        self.source_impl.as_ref().map(|c| match c {
            Suberror::Previous(c) => &**c as &dyn std::error::Error,
            Suberror::Constituent(c) => &**c as &dyn std::error::Error,
        })
    }
}

// Display only the message associated w/ the specifics.
// Consider the rest to be management baggage.
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.specifics)
    }
}
