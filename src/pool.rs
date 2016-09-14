extern crate dbus;

use std::sync::Arc;
use std::cell::Cell;


use dbus::{tree, Path};

#[derive(Debug)]
pub struct Spool {
    pub name: String,
    pub path: Path<'static>,
    pub index: i32,
    pub online: Cell<bool>,
    pub checking: Cell<bool>,
}


#[derive(Copy, Clone, Default, Debug)]
pub struct TData;
impl tree::DataType for TData {
    type ObjectPath = Arc<Spool>;
    type Property = ();
    type Interface = ();
    type Method = ();
    type Signal = (); 
}


impl Spool {

    pub fn new_spool(index: i32, new_name: String) -> Spool {
        Spool {
            name: new_name,
            // TODO use a constant for object path
            path: format!("/org/storage/stratis/{}", index).into(),
            index: index,
            online: Cell::new(index % 2 == 0),
            checking: Cell::new(false),
        }
    } 
}