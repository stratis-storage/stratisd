use pkg_config::Config;
use semver::Version;

fn main() {
    let library = match Config::new()
        .atleast_version("2.2.0")
        .probe("libcryptsetup")
    {
        Ok(l) => l,
        Err(e) => panic!("stratisd requires at least libcryptsetup-2.2.0: {}", e),
    };
    let version = Version::parse(&library.version).unwrap();
    if version < Version::new(2, 3, 0) {
        println!("cargo:rustc-cfg=cryptsetup_compat");
    }
}
