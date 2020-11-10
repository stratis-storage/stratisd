// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{env, error::Error, str::FromStr};

use clap::{App, Arg};
use env_logger::Builder;
use log::LevelFilter;

use libstratis::stratis::run;

fn parse_args() -> App<'static, 'static> {
    App::new("stratisd-min")
        .arg(
            Arg::with_name("log_level")
                .empty_values(false)
                .long("--log-level")
                .possible_values(&["trace", "debug", "info", "warn", "error"])
                .help("Sets level for generation of log messages."),
        )
        .arg(
            Arg::with_name("sim")
                .long("--sim")
                .takes_value(false)
                .help("Enables sim engine."),
        )
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut app = parse_args();
    let mut help = Vec::new();
    app.write_long_help(&mut help)?;
    let help = String::from_utf8(help)?;
    let args = app.get_matches();

    let mut builder = Builder::new();
    if let Some(log_level) = args.value_of("log_level") {
        builder.filter(
            Some("stratisd"),
            LevelFilter::from_str(log_level)
                .expect("argument parser only accepts valid log levels"),
        );
        builder.filter(
            Some("libstratis"),
            LevelFilter::from_str(log_level)
                .expect("argument parser only accepts valid log levels"),
        );
    } else if let Ok(s) = env::var("RUST_LOG") {
        builder.parse_filters(&s);
    }
    builder.init();

    if args.is_present("-h") {
        println!("{}", help);
        Ok(())
    } else {
        run(args.is_present("sim"))?;
        Ok(())
    }
}
