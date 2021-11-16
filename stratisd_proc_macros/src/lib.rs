// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::iter::once;

use proc_macro::TokenStream;
use proc_macro2::{Group, Span, TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::{
    parse, parse_str, punctuated::Punctuated, token::Comma, Attribute, Block, FnArg, Ident,
    ImplItem, ImplItemMethod, Item, Lit, Meta, MetaList, NestedMeta, Pat, PatIdent, PatType, Path,
    PathSegment, Receiver, ReturnType, Stmt, Token, Type, TypePath,
};

/// Add guard for mutating actions when the pool is in maintenance mode.
///
/// This method adds a statement that returns an error if the pool is set
/// to limit available actions.
fn add_method_guards(method: &mut ImplItemMethod, level: Ident) {
    let stmt = if let ReturnType::Type(_, ty) = &method.sig.output {
        if let Type::Path(TypePath {
            path: Path { segments, .. },
            ..
        }) = &**ty
        {
            if let Some(PathSegment { ident, .. }) = segments.iter().last() {
                if &ident.to_string() == "StratisResult" {
                    parse::<Stmt>(TokenStream::from(quote! {
                        if self.action_avail >= crate::engine::types::ActionAvailability::#level {
                            return Err(crate::stratis::StratisError::Msg(format!(
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

/// Process the arguments in the argument list. There is special handling for
/// receivers. Typed `self` parameters are not currently supported in this macro
/// but support can be added if needed.
fn process_arguments(fn_arg: &FnArg) -> (Ident, PatType) {
    match fn_arg {
        FnArg::Receiver(Receiver {
            reference,
            mutability,
            ..
        }) => {
            let reference = reference.as_ref().map(|(r, lt)| quote! { #r #lt });

            let mutability = mutability.map(|m| quote! { #m });

            let self_ident = Ident::new("self", Span::call_site());
            let pool_ident = Ident::new("pool", Span::call_site());

            (
                self_ident,
                PatType {
                    attrs: Vec::new(),
                    pat: Box::new(Pat::from(PatIdent {
                        attrs: Vec::new(),
                        by_ref: None,
                        mutability: None,
                        ident: pool_ident,
                        subpat: None,
                    })),
                    colon_token: Token![:](Span::call_site()),
                    ty: Box::new(
                        parse::<Type>(TokenStream::from(quote! {
                            #reference #mutability crate::engine::strat_engine::pool::StratPool
                        }))
                        .expect("Valid type"),
                    ),
                },
            )
        }
        FnArg::Typed(typed) => {
            let ident = if let Pat::Ident(PatIdent { ref ident, .. }) = *typed.pat {
                if *ident == "self" {
                    panic!("Typed self parameters are not currently supported");
                } else {
                    ident.clone()
                }
            } else {
                panic!("Unrecognized argument format");
            };

            (ident, typed.clone())
        }
    }
}

/// Replace all uses of the `self` keyword with the ident `pool`. This is necessary
/// for wrapped methods as they do not take a `self` reference.
fn process_token_stream(token: TokenTree) -> TokenTree {
    if let TokenTree::Ident(ref i) = token {
        if *i == "self" {
            TokenTree::from(Ident::new("pool", Span::call_site()))
        } else {
            token
        }
    } else if let TokenTree::Group(grp) = token {
        let delimiter = grp.delimiter();
        let tstream = grp
            .stream()
            .into_iter()
            .map(process_token_stream)
            .collect::<TokenStream2>();
        TokenTree::Group(Group::new(delimiter, tstream))
    } else {
        token
    }
}

/// Take the body of the method and wrap it in an inner method, replacing the
/// body with a check for an error that limits available actions.
fn wrap_method(f: &mut ImplItemMethod) {
    let wrapped_ident = Ident::new(
        format!("{}_wrapped", f.sig.ident.clone()).as_str(),
        Span::call_site(),
    );
    let mut wrapped_sig = f.sig.clone();
    wrapped_sig.ident = wrapped_ident.clone();

    let (args, arg_idents) = f.sig.inputs.iter().map(process_arguments).fold(
        (Vec::new(), Vec::new()),
        |(mut args, mut arg_idents), (ident, arg)| {
            args.push(arg);
            arg_idents.push(ident);
            (args, arg_idents)
        },
    );
    let stmts = f.block.stmts.drain(..).collect::<Vec<_>>();

    wrapped_sig.inputs = args
        .into_iter()
        .map(FnArg::Typed)
        .collect::<Punctuated<FnArg, Comma>>();

    let method_body_tokens = quote! {
        #( #stmts )*
    }
    .into_iter()
    .map(process_token_stream)
    .collect::<TokenStream2>();

    let stmt = parse::<Stmt>(TokenStream::from(quote! {
        #wrapped_sig {
            #method_body_tokens
        }
    }))
    .expect("Could not parse generated method as a statement");

    let tokens = quote! { {
        #stmt

        match #wrapped_ident(#( #arg_idents),*) {
            Ok(ret) => Ok(ret),
            Err(e) => {
                if let Some(state) = e.error_to_available_actions() {
                    self.action_avail = state;
                }
                Err(e)
            }
        }
    } };
    f.block = parse::<Block>(TokenStream::from(tokens))
        .expect("Could not parse generated method body as a block");
}

/// Get the pool available action state level at which a pool operation ceases to be
/// accepted.
fn get_attr_level(attrs: &mut Vec<Attribute>) -> Option<Ident> {
    let mut return_value = None;
    let mut index = None;
    for (i, attr) in attrs.iter().enumerate() {
        if let Meta::List(MetaList {
            ref path,
            ref nested,
            ..
        }) = attr
            .parse_meta()
            .unwrap_or_else(|_| panic!("Attribute {:?} cannot be parsed", attr))
        {
            if path
                == &parse_str("pool_mutating_action").expect("pool_mutating_action is valid path")
            {
                for nested_meta in nested.iter() {
                    if let NestedMeta::Lit(Lit::Str(litstr)) = nested_meta {
                        index = Some(i);
                        return_value =
                            Some(parse_str::<Ident>(&litstr.value()).unwrap_or_else(|_| {
                                panic!("{} is not a valid identifier", litstr.value())
                            }));
                    } else {
                        panic!("pool_mutating_action attribute must be in form #[pool_mutating_action(\"REJECTION LEVEL\")]");
                    }
                }
            }
        }
    }
    if let Some(i) = index {
        attrs.remove(i);
    }
    return_value
}

/// Determine whether a method has the given attribute.
fn has_attribute(attrs: &mut Vec<Attribute>, attribute: &str) -> bool {
    let mut return_value = false;
    let mut index = None;
    for (i, attr) in attrs.iter().enumerate() {
        if let Meta::Path(path) = attr
            .parse_meta()
            .unwrap_or_else(|_| panic!("Attribute {:?} cannot be parsed", attr))
        {
            if path == parse_str(attribute).expect("pool_mutating_action is valid path") {
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

/// Determine whether a method should be marked as needing to handle failed rollback
/// based on the attributes.
///
/// The attribute that will cause a method to be marked as potentially causing a failed
/// rollback is `#[pool_rollback]`.
fn performs_rollback(attrs: &mut Vec<Attribute>) -> bool {
    has_attribute(attrs, "pool_rollback")
}

/// Process impl item that was provided to the attribute procedural macro.
fn process_item(mut item: Item) -> Item {
    let i = match item {
        Item::Impl(ref mut i) => i,
        _ => panic!("This macro can only be applied to impl items"),
    };

    for impl_item in i.items.iter_mut() {
        if let ImplItem::Method(ref mut f) = impl_item {
            if let Some(level) = get_attr_level(&mut f.attrs) {
                add_method_guards(f, level);
            }

            if performs_rollback(&mut f.attrs) {
                wrap_method(f);
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
