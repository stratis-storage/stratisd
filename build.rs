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
}
