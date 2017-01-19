// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate rustc_serialize;
extern crate env_logger;

use self::rustc_serialize::json::Json;
use std::env;
use std::fs::File;
use std::io::prelude::Read;
use util::test_consts::OK_TO_DESTROY_DEV_KEY;
use util::test_result::TestError;
use util::test_result::TestErrorEnum;

use util::test_result::TestResult;

pub struct TestConfig {
    filename: String,
    config_string: Option<Json>,
}

impl TestConfig {
    pub fn new(file: &str) -> TestConfig {
        TestConfig {
            filename: String::from(file),
            config_string: None,
        }
    }

    pub fn init(&mut self) -> TestResult<()> {

        env_logger::init().unwrap();

        match self.read_test_config() {
            Ok(_) => return Ok(()),
            Err(_) => {
                return Err(TestError::Framework(TestErrorEnum::Error("Failed to read test config"
                    .into())))
            }
        }

    }

    pub fn get_destroy_devs() {
        match get_safe_to_destroy_devs() {
            Ok(devs) => {
                if devs.len() == 0 {
                    warn!("Test not run.  No available devices.");
                    return;
                }
                devs
            }
            Err(e) => {
                error!("Failed to read safe to destroy device list from config file. {}",
                       e);
                assert!(false);
                return;
            }
        }
    }


    fn get_safe_to_destroy_devs(&mut self) -> TestResult<Vec<String>> {

        if self.config_string.is_none() {
            return Err(TestError::Framework(TestErrorEnum::JsonError("Array not found".into())));
        }

        let json_str = self.config_string.as_ref().unwrap();

        let json_path = match json_str.find_path(&[OK_TO_DESTROY_DEV_KEY]) {
            Some(path) => path,
            None => {
                return Err(TestError::Framework(TestErrorEnum::JsonError("Array not found".into())))
            }
        };

        let mut ok_to_destroy_dev_list = Vec::new();

        match json_path.as_array() {
            Some(array) => {
                for path in array.iter() {
                    match path.as_string() {
                        Some(p) => ok_to_destroy_dev_list.push(String::from(p)),
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

        Ok(ok_to_destroy_dev_list)
    }

    // read the JSON from the config file and store in config_string
    fn read_test_config(&mut self) -> TestResult<()> {

        debug!("start read_test_config() {} {}",
               &self.filename,
               env::current_dir().unwrap().display());

        let mut file = try!(File::open(self.filename.clone()));
        let mut data = String::new();
        try!(file.read_to_string(&mut data));
        debug!("{}", &data);
        self.config_string = match Json::from_str(&data) {
            Ok(j) => Some(j),
            Err(e) => {
                let message = format!("Failed to read JSON {:?}", e);
                return Err(TestError::Framework(TestErrorEnum::JsonError(message)));
            }

        };

        Ok(())
    }
}
