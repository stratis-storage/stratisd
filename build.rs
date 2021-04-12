#[cfg(feature = "systemd_compat")]
use std::{env, path::PathBuf};

#[cfg(feature = "systemd_compat")]
use bindgen::Builder;
use pkg_config::Config;

fn main() {
    if let Err(e) = Config::new().atleast_version("2.3.0").find("libcryptsetup") {
        panic!(
            "At least version 2.3.0 of cryptsetup is required to compile stratisd: {}",
            e
        );
    }

    if let Err(e) = Config::new().atleast_version("2.32.0").find("blkid") {
        panic!(
            "At least version 2.32.0 of blkid is required to compile stratisd: {}",
            e
        );
    }

    #[cfg(feature = "systemd_compat")]
    {
        let bindings = Builder::default()
            .header("/usr/include/systemd/sd-daemon.h")
            .header("/usr/include/systemd/sd-journal.h")
            .generate()
            .expect("Could not generate bindings for systemd");

        let mut path = PathBuf::from(env::var("OUT_DIR").unwrap());
        path.push("bindings.rs");
        bindings
            .write_to_file(&path)
            .expect("Failed to write bindings to file");

        println!("cargo:rustc-link-lib=systemd");
    }
}
