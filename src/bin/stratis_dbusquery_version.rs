use dbus::{
    self,
    arg::{RefArg, Variant},
    blocking::Connection,
};
use lazy_static::lazy_static;
use semver::Version;
use std::time::Duration;

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

fn main() {
    let vertest = get_version();
    println!("{:#?}", vertest);
    let verparse = (Version::parse(vertest.unwrap().as_str().unwrap())).unwrap();
    println!("verparse: {:#?}", verparse);
    println!("STRATIS_VER_UDEV_SYMLINK: {:#?}", *STRATIS_VER_UDEV_SYMLINK);
    assert!(verparse >= *STRATIS_VER_UDEV_SYMLINK);
}
