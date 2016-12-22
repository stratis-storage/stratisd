extern crate rustc_serialize;

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Error, Write};
use std::path::Path;

use test_consts::*;

use stratisd::engine::EngineError;
use stratisd::consts::SECTOR_SIZE;

use rustc_serialize::json::{Json, ParserError};

#[derive(Debug)]
pub enum TestErrorEnum {
    _Ok,
    Error(String),
    JsonError(String),
    _NotFound(String),
}

impl TestErrorEnum {
    pub fn get_error_string(&self) -> String {
        match *self {
            TestErrorEnum::_Ok => "Ok".into(),
            TestErrorEnum::Error(ref x) => format!("{}", x),
            TestErrorEnum::JsonError(ref x) => format!("{} already exists", x),
            TestErrorEnum::_NotFound(ref x) => format!("{} is not found", x),
        }
    }
}

impl fmt::Display for TestErrorEnum {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.get_error_string())
    }
}

// Define a common error enum.
// See http://blog.burntsushi.net/rust-error-handling/
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
            TestError::Framework(ref err) => write!(f, "Framework error: {}", err),
            TestError::EngineError(ref err) => write!(f, "Engine error: {:?}", err),
            TestError::Json(ref err) => write!(f, "Json error: {}", err),
            TestError::Io(ref err) => write!(f, "IO error: {}", err),
        }
    }
}

pub type TestResult<T> = Result<T, TestError>;

pub struct TestConfig {
    json: Json,
}

impl TestConfig {
    pub fn new(config_json: Json) -> TestConfig {
        TestConfig { json: config_json }
    }

    pub fn get_safe_to_destroy_devs(&mut self) -> TestResult<Option<Vec<&str>>> {

        let json_path = match self.json.find_path(&[OK_TO_DESTROY_DEV_ARRAY]) {
            Some(path) => path,
            None => {
                return Err(TestError::Framework(TestErrorEnum::JsonError("Array not found".into())))
            }
        };

        let mut devices = Vec::new();

        match json_path.as_array() {
            Some(array) => {
                for path in array.iter() {
                    match path.as_string() {
                        Some(p) => devices.push(p.clone()),
                        None => {
                    return Err(TestError::Framework(TestErrorEnum::JsonError("Path conversion \
                                                                              failed"
                        .into())))
                }
                    }
                }
            }
            None => {
                return Err(TestError::Framework(TestErrorEnum::JsonError("Array conversion \
                                                                          failed"
                    .into())))
            }
        };

        Ok(Some(devices))
    }
}

fn check_valid_blockdev_paths(blockdev_paths: &Vec<&Path>) -> TestResult<()> {
    for path in blockdev_paths {
        if path.exists() == false {
            panic!("invalid path to blockdev {}", path.to_string_lossy());
        }
    }

    Ok(())
}

pub fn get_default_blockdevs(config: &mut TestConfig) -> TestResult<Vec<&Path>> {

    let safe_to_destroy_devs: Vec<&Path> = match config.get_safe_to_destroy_devs() {
        Ok(option) => {
            match option {
                Some(devs) => devs.iter().map(|x| Path::new(x.clone())).collect::<Vec<&Path>>(),
                None => panic!("no devs supplied for testing"),
            }
        }
        Err(err) => panic!("failed to read devs {:?}", err),
    };

    match check_valid_blockdev_paths(&safe_to_destroy_devs) {
        Ok(_) => {}
        Err(err) => {
            return Err(err);
        }
    }
    Ok(safe_to_destroy_devs)
}

pub fn read_test_config(config_file: &str) -> TestResult<TestConfig> {

    let mut file = try!(File::open(config_file));
    let mut data = String::new();
    try!(file.read_to_string(&mut data));

    let json = try!(Json::from_str(&data));

    Ok(TestConfig::new(json))
}

pub fn clean_blockdevs_headers(blockdev_paths: &Vec<&Path>) -> TestResult<()> {
    for path in blockdev_paths {
        match wipe_sigblock(path) {
            Ok(_) => {}
            Err(err) => {
                panic!("Failed to clean header on {:?} : {:?}", path, err);
            }
        }
    }

    Ok(())
}

pub fn wipe_sigblock(path: &Path) -> TestResult<()> {
    let zeroed = [0u8; (SECTOR_SIZE * 8) as usize];
    let mut f = try!(OpenOptions::new().write(true).open(path));
    try!(f.write_all(&zeroed[..SECTOR_SIZE as usize]));
    try!(f.flush());
    Ok(())
}
