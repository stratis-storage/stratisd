use std::{process, time::Duration};

use dbus::{
    self,
    arg::{RefArg, Variant},
    blocking::Connection,
};
use lazy_static::lazy_static;
use semver::Version;

const DBUS_PROPS_IFACE: &str = "org.freedesktop.DBus.Properties";
const STRATIS_BUS_NAME: &str = "org.storage.stratis2";
const STRATIS_MANAGER_OBJECT: &str = "/org/storage/stratis2";
const STRATIS_MANAGER_IFACE: &str = "org.storage.stratis2.Manager.r1";
lazy_static! {
    static ref TIMEOUT: Duration = Duration::new(5, 0);
}
lazy_static! {
    static ref STRATIS_VER_UDEV_SYMLINK: Version =
        Version::parse("2.2.0").expect("version string is well-formed");
}

type GetVerRet = Variant<Box<dyn RefArg + 'static>>;

fn get_version() -> Result<GetVerRet, dbus::Error> {
    let connection = Connection::new_system()?;
    let proxy = connection.with_proxy(STRATIS_BUS_NAME, STRATIS_MANAGER_OBJECT, *TIMEOUT);
    Ok(proxy
        .method_call(DBUS_PROPS_IFACE, "Get", (STRATIS_MANAGER_IFACE, "Version"))
        .map(|r: (GetVerRet,)| r.0)?)
}

fn run() -> Result<Version, String> {
    let vertest = get_version().map_err(|vertest_err| format!("vertest_err: {}", vertest_err))?;
    println!("{:#?}", vertest);
    let verparse = Version::parse(
        vertest
            .as_str()
            .ok_or_else(|| "no version info".to_string())?,
    )
    .map_err(|verparse_err| format!("verparse_err: {}", verparse_err))?;
    println!("verparse: {:#?}", verparse);
    println!("STRATIS_VER_UDEV_SYMLINK: {:#?}", *STRATIS_VER_UDEV_SYMLINK);
    Ok(verparse)
}

fn main() {
    match run() {
        Ok(version) => {
            if version < *STRATIS_VER_UDEV_SYMLINK {
                eprintln!("stratisd version does not support symlinks in /dev/stratis");
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Could not obtain version from stratisd: {}", e);
            process::exit(2);
        }
    };
}
