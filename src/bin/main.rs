// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_use(crate_version)]
extern crate clap;
extern crate term;

extern crate stratisd;

use std::process::exit;

use clap::App;
use clap::Arg;

use stratisd::get_engine;
use stratisd::run;
use stratisd::set_debug;
use stratisd::write_err;


fn main() {
    let matches = App::new("stratis")
        .version(&crate_version!())
        .about("Stratis storage management")
        .arg(Arg::with_name("debug")
            .long("debug")
            .help("Print additional output for debugging"))
        .arg(Arg::with_name("sim")
            .long("sim")
            .help("Use simulator engine"))
        .get_matches();

    set_debug(matches.is_present("debug"));

    let r = run(get_engine(matches.is_present("sim")));

    if let Err(r) = r {
        if let Err(e) = write_err(r) {
            panic!("Unable to write to stderr: {}", e)
        }

        exit(1);
    }
}
