use std::collections::HashMap;

use dbus::{
    arg::{Array, RefArg, Variant},
    tree::{MTFn, MethodInfo, MethodResult},
    Message,
};
use itertools::Itertools;

use crate::dbus_api::{
    consts,
    pool::pool_operation,
    types::TData,
    util::{get_next_arg, result_to_tuple},
};

fn get_properties_shared(
    m: &MethodInfo<MTFn<TData>, TData>,
    properties: &mut dyn Iterator<Item = String>,
) -> MethodResult {
    let message: &Message = m.msg;
    let object_path = &m.path;

    let return_message = message.method_return();

    let return_value: HashMap<String, (bool, Variant<Box<dyn RefArg>>)> = properties
        .unique()
        .filter_map(|prop| match prop.as_str() {
            consts::POOL_TOTAL_SIZE_PROP => Some((
                prop,
                pool_operation(m.tree, object_path.get_name(), |(_, _, pool)| {
                    Ok((u128::from(*pool.total_physical_size())
                        * devicemapper::SECTOR_SIZE as u128)
                        .to_string())
                }),
            )),
            consts::POOL_TOTAL_USED_PROP => Some((
                prop,
                pool_operation(m.tree, object_path.get_name(), |(_, _, pool)| {
                    pool.total_physical_used()
                        .map_err(|e| e.to_string())
                        .map(|size| {
                            (u128::from(*size) * devicemapper::SECTOR_SIZE as u128).to_string()
                        })
                }),
            )),
            _ => None,
        })
        .map(|(key, result)| result_to_tuple(key, result))
        .collect();

    Ok(vec![return_message.append1(return_value)])
}

pub fn get_all_properties(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    get_properties_shared(
        m,
        &mut vec![consts::POOL_TOTAL_SIZE_PROP, consts::POOL_TOTAL_USED_PROP]
            .into_iter()
            .map(|s| s.to_string()),
    )
}

pub fn get_properties(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let mut properties: Array<String, _> = get_next_arg(&mut iter, 0)?;
    get_properties_shared(m, &mut properties)
}
