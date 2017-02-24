// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
extern crate rustc_serialize;
extern crate stratis;

use stratis::engine::EngineError;
use self::rustc_serialize::json::ParserError;
use std::fmt;
use std::io::Error;

#[derive(Debug)]
pub enum TestErrorEnum {
    Error(String),
    JsonError(String),
}

impl TestErrorEnum {
    pub fn get_error_string(&self) -> String {
        match *self {
            TestErrorEnum::Error(ref x) => format!("{}", x),
            TestErrorEnum::JsonError(ref x) => format!("{} already exists", x),
        }
    }
}

impl fmt::Display for TestErrorEnum {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.get_error_string())
    }
}

#[derive(Debug)]
pub enum TestError {
    Framework(TestErrorEnum),
    EngineError(EngineError),
    Json(ParserError),
    Io(Error),
}

impl From<TestErrorEnum> for TestError {
    fn from(err: TestErrorEnum) -> TestError {
        TestError::Framework(err)
    }
}

impl From<EngineError> for TestError {
    fn from(err: EngineError) -> TestError {
        TestError::EngineError(err)
    }
}

impl From<Error> for TestError {
    fn from(err: Error) -> TestError {
        TestError::Io(err)
    }
}

impl From<ParserError> for TestError {
    fn from(err: ParserError) -> TestError {
        TestError::Json(err)
    }
}

impl fmt::Display for TestError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            TestError::Framework(ref err) => write!(f, "Test error: {}", err),
            TestError::EngineError(ref err) => write!(f, "Engine error: {:?}", err),
            TestError::Json(ref err) => write!(f, "Json error: {}", err),
            TestError::Io(ref err) => write!(f, "IO error: {}", err),
        }
    }
}

pub type TestResult<T> = Result<T, TestError>;
