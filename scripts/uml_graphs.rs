//!
//! ```cargo
//!    [dependencies]
//!    mml="0.1"
//!    walkdir = "2"
//! ```
extern crate mml;
extern crate walkdir;

use std::ffi::OsStr;
use std::path::PathBuf;

use walkdir::WalkDir;

/// Generate UML diagrams for all modules in the code base.
fn main() {
    let base_target = PathBuf::from("target/doc");
    let base_src = PathBuf::from("src");

    for entry in WalkDir::new(&base_src)
            .into_iter()
            .filter_entry(|e| {
        let file_type = e.file_type();
        let path = e.path();
        let file_stem = path.file_stem();
        file_type.is_dir() ||
        (file_type.is_file() && path.extension() == Some(OsStr::new("rs")) &&
         !(file_stem == Some(OsStr::new("lib"))) &&
         !(file_stem == Some(OsStr::new("mod"))))
    })
            .filter(|e| e.is_ok())
            .map(|e| e.expect("must be ok")) {
        let src = PathBuf::from(entry.path());

        let mut src_components = src.components();
        src_components.next();

        let mut target_path = PathBuf::from(src_components.as_path());
        target_path.set_extension("");

        let target: PathBuf = [&base_target, &target_path].iter().collect();
        let _ = mml::src2both(&src, &target);
    }
}
