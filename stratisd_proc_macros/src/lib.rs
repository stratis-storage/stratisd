// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::iter::once;

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse, parse_str, Attribute, ImplItem, ImplItemMethod, Item, Meta, Path, PathSegment,
    ReturnType, Stmt, Type, TypePath,
};

/// Add guard for mutating actions when the pool is in maintenance mode.
fn add_method_guards(method: &mut ImplItemMethod) {
    let stmt = if let ReturnType::Type(_, ty) = &method.sig.output {
        if let Type::Path(TypePath {
            path: Path { segments, .. },
            ..
        }) = &**ty
        {
            if let Some(PathSegment { ident, .. }) = segments.iter().last() {
                if &ident.to_string() == "StratisResult" {
                    parse::<Stmt>(TokenStream::from(quote! {
                        if self.action_avail != crate::engine::types::ActionAvailability::Full {
                            return Err(crate::stratis::StratisError::Error(format!(
                                "Pool is in state {:?} where mutable actions cannot be performed until the issue is resolved manually",
                                self.action_avail
                            )));
                        }
                    })).expect("This block should be a valid statement")
                } else {
                    panic!("The only return type currently supported for mutable actions is StratisResult<_>; found return type {} for method {}", ident.to_string(), method.sig.ident);
                }
            } else {
                unreachable!();
            }
        } else {
            panic!(
                "Could not check return type for method {}",
                method.sig.ident
            );
        }
    } else {
        panic!(
            "Mutable action methods must return a value; method {} has not return type",
            method.sig.ident
        );
    };
    let stmts = once(stmt)
        .chain(method.block.stmts.iter().cloned())
        .collect();
    method.block.stmts = stmts;
}

/// Determine whether a method should be marked as a mutating action based on the attributes.
///
/// The attribute that will cause a method to be marked as performing a mutating action is
/// `#[pool_mutating_action]`.
fn is_mutating_action(attrs: &mut Vec<Attribute>) -> bool {
    let mut return_value = false;
    let mut index = None;
    for (i, attr) in attrs.iter().enumerate() {
        if let Meta::Path(path) = attr
            .parse_meta()
            .expect(&format!("Attribute {:?} cannot be parsed", attr))
        {
            if path
                == parse_str("pool_mutating_action").expect("pool_mutating_action is valid path")
            {
                index = Some(i);
                return_value = true;
            }
        }
    }
    if let Some(i) = index {
        attrs.remove(i);
    }
    return_value
}

/// Process impl item that was provided to the attribute procedural macro.
fn process_item(mut item: Item) -> Item {
    let i = match item {
        Item::Impl(ref mut i) => i,
        _ => panic!("This macro can only be applied to impl items"),
    };

    for impl_item in i.items.iter_mut() {
        if let ImplItem::Method(ref mut f) = impl_item {
            if is_mutating_action(&mut f.attrs) {
                add_method_guards(f);
            }
        }
    }

    item
}

/// This macro is specifically targeted to remove boilerplate code in the StratPool
/// implementations. It provides two facilities:
/// * checking if the error returned should cause the pool to refuse mutating actions.
/// * returning an error if the method called would cause a mutating action to occur if
/// the pool cannot accept mutating actions.
///
/// This macro should be applied to `impl` items only.
#[proc_macro_attribute]
pub fn strat_pool_impl_gen(_: TokenStream, item: TokenStream) -> TokenStream {
    let item = process_item(parse::<Item>(item).expect("Could not parse input as item"));
    TokenStream::from(quote! {
        #item
    })
}
