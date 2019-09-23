use std::collections::HashSet;

use proc_macro2::Span;
use syn::{
    bracketed,
    parse::{self, Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    Abi, AttrStyle, Attribute, Expr, FnArg, ForeignItemFn, Ident, Item, ItemFn, ItemStatic, LitInt,
    Pat, PatType, PathArguments, ReturnType, Stmt, Token, Type, Visibility,
};

use crate::{ast::Access, Map, Set};

pub fn abi_is_c(abi: &Abi) -> bool {
    match &abi.name {
        None => true,
        Some(s) => s.value() == "C",
    }
}

pub fn attr_eq(attr: &Attribute, name: &str) -> bool {
    attr.style == AttrStyle::Outer && attr.path.segments.len() == 1 && {
        let segment = attr.path.segments.first().unwrap();
        segment.arguments == PathArguments::None && segment.ident.to_string() == name
    }
}

/// checks that a function signature
///
/// - has no bounds (like where clauses)
/// - is not `async`
/// - is not `const`
/// - is not `unsafe`
/// - is not generic (has no type parametrs)
/// - is not variadic
/// - uses the Rust ABI (and not e.g. "C")
pub fn check_fn_signature(item: &ItemFn) -> bool {
    item.vis == Visibility::Inherited
        && item.sig.constness.is_none()
        && item.sig.asyncness.is_none()
        && item.sig.abi.is_none()
        && item.sig.unsafety.is_none()
        && item.sig.generics.params.is_empty()
        && item.sig.generics.where_clause.is_none()
        && item.sig.variadic.is_none()
}

pub fn check_foreign_fn_signature(item: &ForeignItemFn) -> bool {
    item.vis == Visibility::Inherited
        // && item.constness.is_none()
        // && item.asyncness.is_none()
        // && item.abi.is_none()
        // && item.unsafety.is_none()
        && item.sig.generics.params.is_empty()
        && item.sig.generics.where_clause.is_none()
        && item.sig.variadic.is_none()
}

pub fn extract_cfgs(attrs: Vec<Attribute>) -> (Vec<Attribute>, Vec<Attribute>) {
    let mut cfgs = vec![];
    let mut not_cfgs = vec![];

    for attr in attrs {
        if attr_eq(&attr, "cfg") {
            cfgs.push(attr);
        } else {
            not_cfgs.push(attr);
        }
    }

    (cfgs, not_cfgs)
}

/// `#[core = 0]`
pub fn extract_core(
    mut attrs: Vec<Attribute>,
    cores: u8,
    span: Span,
) -> parse::Result<(u8, Vec<Attribute>)> {
    struct Rhs {
        _eq: Token![=],
        lit: LitInt,
    }

    impl Parse for Rhs {
        fn parse(input: ParseStream<'_>) -> parse::Result<Self> {
            Ok(Rhs {
                _eq: input.parse()?,
                lit: input.parse()?,
            })
        }
    }

    let mut res = None;
    for (pos, attr) in attrs.iter().enumerate() {
        if attr_eq(attr, "core") {
            if let Ok(rhs) = syn::parse2::<Rhs>(attr.tokens.clone()) {
                if rhs.lit.suffix().is_empty() {
                    let core = rhs.lit.base10_parse::<u8>()?;

                    if core < cores {
                        res = Some((pos, core));
                        break;
                    }
                }
            }
        }
    }

    let (pos, core) = res.ok_or_else(|| {
        parse::Error::new(
            span,
            "core needs to be specified using the `#[core = 0]` attribute",
        )
    })?;

    attrs.remove(pos);

    Ok((core, attrs))
}

pub fn extract_locals(stmts: Vec<Stmt>) -> parse::Result<(Vec<ItemStatic>, Vec<Stmt>)> {
    let mut istmts = stmts.into_iter();

    let mut seen = HashSet::new();
    let mut locals = vec![];
    let mut stmts = vec![];
    while let Some(stmt) = istmts.next() {
        match stmt {
            Stmt::Item(Item::Static(static_)) => {
                if static_.mutability.is_some() {
                    if seen.contains(&static_.ident) {
                        return Err(parse::Error::new(
                            static_.ident.span(),
                            "this local `static` appears more than once",
                        ));
                    }

                    seen.insert(static_.ident.clone());
                    locals.push(static_);
                } else {
                    stmts.push(Stmt::Item(Item::Static(static_)));
                    break;
                }
            }

            _ => {
                stmts.push(stmt);
                break;
            }
        }
    }

    stmts.extend(istmts);

    Ok((locals, stmts))
}

pub fn extract_shared(attrs: &mut Vec<Attribute>, cores: u8) -> parse::Result<bool> {
    if let Some(pos) = attrs.iter().position(|attr| attr_eq(attr, "shared")) {
        if cores == 1 {
            Err(parse::Error::new(
                attrs[pos].span(),
                "`#[shared]` can only be used in multi-core mode",
            ))
        } else {
            attrs.remove(pos);

            Ok(true)
        }
    } else {
        Ok(false)
    }
}

pub fn parse_core(lit: LitInt, cores: u8) -> parse::Result<u8> {
    if !lit.suffix().is_empty() {
        return Err(parse::Error::new(
            lit.span(),
            "this integer must be unsuffixed",
        ));
    }

    if let Ok(val) = lit.base10_parse::<u8>() {
        if val < cores {
            return Ok(val);
        }
    }

    Err(parse::Error::new(
        lit.span(),
        &format!("core number must be in the range 0..{}", cores),
    ))
}

pub fn parse_idents(content: ParseStream<'_>) -> parse::Result<Set<Ident>> {
    let inner;
    bracketed!(inner in content);

    let mut idents = Set::new();
    for ident in inner.call(Punctuated::<Ident, Token![,]>::parse_terminated)? {
        if idents.contains(&ident) {
            return Err(parse::Error::new(
                ident.span(),
                "identifier appears more than once in list",
            ));
        }

        idents.insert(ident);
    }

    Ok(idents)
}

pub fn parse_resources(content: ParseStream<'_>) -> parse::Result<Map<Access>> {
    let inner;
    bracketed!(inner in content);

    let mut resources = Map::new();
    for e in inner.call(Punctuated::<Expr, Token![,]>::parse_terminated)? {
        let err = Err(parse::Error::new(
            e.span(),
            "identifier appears more than once in list",
        ));
        let (access, path) = match e {
            Expr::Path(e) => (Access::Exclusive, e.path),

            Expr::Reference(ref r) if r.mutability.is_none() => match &*r.expr {
                Expr::Path(e) => (Access::Shared, e.path.clone()),

                _ => return err,
            },

            _ => return err,
        };

        let ident = if path.leading_colon.is_some()
            || path.segments.len() != 1
            || path.segments[0].arguments != PathArguments::None
        {
            return Err(parse::Error::new(
                path.span(),
                "resource must be an identifier, not a path",
            ));
        } else {
            path.segments[0].ident.clone()
        };

        if resources.contains_key(&ident) {
            return Err(parse::Error::new(
                ident.span(),
                "resource appears more than once in list",
            ));
        }

        resources.insert(ident, access);
    }

    Ok(resources)
}

pub fn parse_inputs(
    inputs: Punctuated<FnArg, Token![,]>,
    name: &str,
) -> Option<(Box<Pat>, Result<Vec<PatType>, FnArg>)> {
    let mut inputs = inputs.into_iter();

    match inputs.next() {
        Some(FnArg::Typed(first)) => {
            if type_is_path(&first.ty, &[name, "Context"]) {
                let rest = inputs
                    .map(|arg| match arg {
                        FnArg::Typed(arg) => Ok(arg),
                        _ => Err(arg),
                    })
                    .collect::<Result<Vec<_>, _>>();

                Some((first.pat, rest))
            } else {
                None
            }
        }

        _ => None,
    }
}

pub fn type_is_bottom(ty: &ReturnType) -> bool {
    if let ReturnType::Type(_, ty) = ty {
        if let Type::Never(_) = **ty {
            true
        } else {
            false
        }
    } else {
        false
    }
}

pub fn type_is_late_resources(ty: &ReturnType, name: &str) -> Result<bool, ()> {
    match ty {
        ReturnType::Default => Ok(false),

        ReturnType::Type(_, ty) => match &**ty {
            Type::Tuple(t) => {
                if t.elems.is_empty() {
                    Ok(false)
                } else {
                    Err(())
                }
            }

            Type::Path(_) => {
                if type_is_path(ty, &[name, "LateResources"]) {
                    Ok(true)
                } else {
                    Err(())
                }
            }

            _ => Err(()),
        },
    }
}

pub fn type_is_path(ty: &Type, segments: &[&str]) -> bool {
    match ty {
        Type::Path(tpath) if tpath.qself.is_none() => {
            tpath.path.segments.len() == segments.len()
                && tpath
                    .path
                    .segments
                    .iter()
                    .zip(segments)
                    .all(|(lhs, rhs)| lhs.ident == **rhs)
        }

        _ => false,
    }
}

// pub fn path_is_segments(path: &syn::Path, segments: &[(&str, &str)]) -> bool {
//     // if let syn::PathArguments::AngleBracketed(gen_args) = path.arguments {
//     //     return true;
//     // }
//     path.segments.len() == segments.len()
//         && path
//             .segments
//             .iter()
//             .zip(segments)
//             .all(|(lhs, rhs)| lhs.ident == rhs.0 && lhs.)
// }

pub fn path_is_generator(path: &syn::Path) -> bool {
    path.segments.len() == 1
        && path.segments[0].ident == "Generator"
        && if let syn::PathArguments::AngleBracketed(abga) = &path.segments[0].arguments {
            let g = &abga.args;
            g.len() == 2
                && match &g[0] {
                    syn::GenericArgument::Binding(b) => {
                        b.ident == "Yield"
                            && match &b.ty {
                                Type::Tuple(e) => e.elems.is_empty(),
                                _ => false,
                            }
                    }
                    _ => false,
                }
                && match &g[1] {
                    syn::GenericArgument::Binding(b) => {
                        b.ident == "Return"
                            && match &b.ty {
                                Type::Never(_) => true,
                                _ => false,
                            }
                    }
                    _ => false,
                }
        } else {
            false
        }
}

pub fn type_is_unit(ty: &ReturnType) -> bool {
    if let ReturnType::Type(_, ty) = ty {
        if let Type::Tuple(ref tuple) = **ty {
            // Ok, () return type
            tuple.elems.is_empty()
        } else {
            // other return type, should be ()
            false
        }
    } else {
        // Ok, no return type
        true
    }
}

pub fn type_is_generator(ty: &ReturnType) -> bool {
    if let ReturnType::Type(_, ty) = ty {
        if let Type::ImplTrait(ref tit) = **ty {
            if tit.bounds.len() == 1 {
                if let Some(syn::TypeParamBound::Trait(tb)) = tit.bounds.first() {
                    path_is_generator(&tb.path)
                        && tb.paren_token == None
                        && tb.modifier == syn::TraitBoundModifier::None
                        && tb.lifetimes == None
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            // wrong return type, not ImplTrait
            false
        }
    } else {
        // no return type, not ImpleTrait
        false
    }
}
