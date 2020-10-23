// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::tree::{Factory, MTSync, Method};

use crate::dbus_api::{
    pool::fetch_properties_2_3::methods::{get_all_properties, get_properties},
    types::TData,
};

pub fn get_all_properties_method(
    f: &Factory<MTSync<TData>, TData>,
) -> Method<MTSync<TData>, TData> {
    f.method("GetAllProperties", (), get_all_properties)
        // a{s(bv)}: Dictionary of property names to tuples
        // In the tuple:
        // b: Indicates whether the property value fetched was successful
        // v: If b is true, represents the value for the given property
        //    If b is false, represents the error returned when fetching the property
        .out_arg(("results", "a{s(bv)}"))
}

pub fn get_properties_method(f: &Factory<MTSync<TData>, TData>) -> Method<MTSync<TData>, TData> {
    f.method("GetProperties", (), get_properties)
        .in_arg(("properties", "as"))
        // a{s(bv)}: Dictionary of property names to tuples
        // In the tuple:
        // b: Indicates whether the property value fetched was successful
        // v: If b is true, represents the value for the given property
        //    If b is false, represents the error returned when fetching the property
        .out_arg(("results", "a{s(bv)}"))
}
