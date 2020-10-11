use std::collections::HashSet;

use indexmap::map::Entry;
use proc_macro2::TokenStream as TokenStream2;
use syn::{
    parse::{self, ParseStream, Parser},
    spanned::Spanned,
    Attribute, ExprParen, Fields, ForeignItem, Ident, Item, Lit, Path, Token, Visibility,
};

use super::Input;
use crate::{
    ast::{
        App, AppArgs, CustomArg, ExternInterrupt, ExternInterrupts, HardwareTask, Idle, IdleArgs,
        Init, InitArgs, LateResource, Resource, SoftwareTask,
    },
    parse::util,
    Either, Map, Settings,
};

impl AppArgs {
    pub(crate) fn parse(tokens: TokenStream2) -> parse::Result<Self> {
        (|input: ParseStream<'_>| -> parse::Result<Self> {
            let mut custom = Map::new();

            loop {
                if input.is_empty() {
                    break;
                }

                // #ident = ..
                let ident: Ident = input.parse()?;
                let _eq_token: Token![=] = input.parse()?;

                let ident_s = ident.to_string();
                match &*ident_s {
                    _ => {
                        if custom.contains_key(&ident) {
                            return Err(parse::Error::new(
                                ident.span(),
                                "argument appears more than once",
                            ));
                        }

                        // Parse as path
                        if let Ok(p) = input.parse::<Path>() {
                            custom.insert(ident, CustomArg::Path(p));
                        } else {
                            // Parse as literal
                            match input.parse::<Lit>()? {
                                Lit::Bool(lit) => {
                                    custom.insert(ident, CustomArg::Bool(lit.value));
                                }
                                Lit::Int(lit) => {
                                    if lit.suffix().is_empty() {
                                        custom.insert(
                                            ident,
                                            CustomArg::UInt(lit.base10_digits().to_string()),
                                        );
                                    } else {
                                        return Err(parse::Error::new(
                                            ident.span(),
                                            "integer must be unsuffixed",
                                        ));
                                    }
                                }
                                _ => {
                                    return Err(parse::Error::new(
                                        ident.span(),
                                        "argument has unexpected value",
                                    ));
                                }
                            }
                        }
                    }
                }

                if input.is_empty() {
                    break;
                }

                // ,
                let _: Token![,] = input.parse()?;
            }

            Ok(AppArgs { custom })
        })
        .parse2(tokens)
    }
}

enum AppAttribute {
    Resources,
    Init,
    Idle,
    Task,
    Dispatch,
}

fn check_attr(attr: &Attribute) -> Option<AppAttribute> {
    if util::attr_eq(attr, "resources") {
        Some(AppAttribute::Resources)
    } else if util::attr_eq(attr, "init") {
        Some(AppAttribute::Init)
    } else if util::attr_eq(attr, "idle") {
        Some(AppAttribute::Idle)
    } else if util::attr_eq(attr, "task") {
        Some(AppAttribute::Task)
    } else if util::attr_eq(attr, "dispatch") {
        Some(AppAttribute::Dispatch)
    } else {
        None
    }
}

fn parse_attr(
    attrs: &mut Vec<Attribute>,
) -> Result<(Option<AppAttribute>, Option<Attribute>), parse::Error> {
    let mut r = (None, None);
    loop {
        if attrs
            .iter()
            .position(|attr| match check_attr(attr) {
                Some(app_attr) => match r {
                    (None, _) => {
                        r.0 = Some(app_attr);
                        true
                    }
                    (Some(_), _) => unimplemented!(),
                },
                None => false,
            })
            .map(|e| r.1 = Some(attrs.remove(e)))
            .is_none()
        {
            break;
        }
    }
    Ok(r)
}

impl App {
    pub(crate) fn parse(args: AppArgs, input: Input, settings: &Settings) -> parse::Result<Self> {
        let mut inits = Vec::new();
        let mut idles = Vec::new();

        let mut late_resources = Map::new();
        let mut resources = Map::new();
        let mut resource_struct = Map::new();
        let mut hardware_tasks = Map::new();
        let mut software_tasks = Map::new();
        let mut user_imports = vec![];
        let mut user_code = vec![];

        let mut extern_interrupts = ExternInterrupts::new();

        let mut seen_idents = HashSet::<Ident>::new();
        let mut bindings = HashSet::<Ident>::new();

        let mut check_binding = |ident: &Ident| {
            if bindings.contains(ident) {
                return Err(parse::Error::new(
                    ident.span(),
                    "a task has already been bound to this interrupt",
                ));
            } else {
                bindings.insert(ident.clone());
            }

            Ok(())
        };

        let mut check_ident = |ident: &Ident| {
            if seen_idents.contains(ident) {
                return Err(parse::Error::new(
                    ident.span(),
                    "this identifier has already been used",
                ));
            } else {
                seen_idents.insert(ident.clone());
            }

            Ok(())
        };

        for mut item in input.items {
            match item {
                Item::Fn(mut item) => {
                    let span = item.sig.ident.span();
                    match parse_attr(&mut item.attrs)? {
                        (Some(AppAttribute::Init), Some(init)) => {
                            // If an init function already exists, error
                            if !inits.is_empty() {
                                return Err(parse::Error::new(
                                    span,
                                    "`#[init]` function must appear at most once",
                                ));
                            }

                            check_ident(&item.sig.ident)?;
                            let args = InitArgs::parse(init.tokens, settings)?;

                            inits.push(Init::parse(args, item)?);
                        }
                        (Some(AppAttribute::Idle), Some(idle)) => {
                            // If an idle function already exists, error
                            if !idles.is_empty() {
                                return Err(parse::Error::new(
                                    span,
                                    "`#[idle]` function must appear at most once",
                                ));
                            }

                            check_ident(&item.sig.ident)?;
                            let args = IdleArgs::parse(idle.tokens, settings)?;

                            idles.push(Idle::parse(args, item)?);
                        }
                        (Some(AppAttribute::Task), Some(task)) => {
                            eprintln!("--- task ---");
                            if hardware_tasks.contains_key(&item.sig.ident)
                                || software_tasks.contains_key(&item.sig.ident)
                            {
                                return Err(parse::Error::new(
                                    span,
                                    "this task is defined multiple times",
                                ));
                            }

                            match crate::parse::task_args(task.tokens, settings)? {
                                Either::Left(args) => {
                                    check_binding(&args.binds)?;
                                    check_ident(&item.sig.ident)?;

                                    hardware_tasks.insert(
                                        item.sig.ident.clone(),
                                        HardwareTask::parse(args, item)?,
                                    );
                                }

                                Either::Right(args) => {
                                    eprintln!("--- software task aaueaeu ---");
                                    check_ident(&item.sig.ident)?;

                                    software_tasks.insert(
                                        item.sig.ident.clone(),
                                        SoftwareTask::parse(args, item)?,
                                    );
                                }
                            }
                        }
                        _ => {
                            return Err(parse::Error::new(
                                span,
                                "this item must live outside the `#[app]` module",
                            ));
                        }
                    }
                }

                Item::Struct(ref mut struct_item) => {
                    // Match structures with the attribute #[resources], name of structure is not
                    // important
                    let span = struct_item.ident.span();
                    match parse_attr(&mut struct_item.attrs)? {
                        (Some(AppAttribute::Resources), _) => {
                            if resource_struct.contains_key(&struct_item.ident) {
                                return Err(parse::Error::new(
                                    span,
                                    "`#[resources]` struct must appear at most once",
                                ));
                            }

                            if struct_item.vis != Visibility::Inherited {
                                return Err(parse::Error::new(
                                    struct_item.span(),
                                    "this item must have inherited / private visibility",
                                ));
                            }

                            if let Fields::Named(fields) = &mut struct_item.fields {
                                for field in &mut fields.named {
                                    let ident = field.ident.as_ref().expect("UNREACHABLE");

                                    if late_resources.contains_key(ident)
                                        || resources.contains_key(ident)
                                    {
                                        return Err(parse::Error::new(
                                            ident.span(),
                                            "this resource is listed more than once",
                                        ));
                                    }

                                    if let Some(pos) = field
                                        .attrs
                                        .iter()
                                        .position(|attr| util::attr_eq(attr, "init"))
                                    {
                                        let attr = field.attrs.remove(pos);

                                        let late = LateResource::parse(field, ident.span())?;

                                        resources.insert(
                                            ident.clone(),
                                            Resource {
                                                late,
                                                expr: syn::parse2::<ExprParen>(attr.tokens)?.expr,
                                            },
                                        );
                                    } else {
                                        let late = LateResource::parse(field, ident.span())?;

                                        late_resources.insert(ident.clone(), late);
                                    }
                                }
                            } else {
                                return Err(parse::Error::new(
                                    struct_item.span(),
                                    "this `struct` must have named fields",
                                ));
                            }
                            // resource_struct will be non-empty if #[resources] was encountered before
                            resource_struct.insert(struct_item.ident.clone(), struct_item.clone());
                        }
                        _ => {
                            // Structure without the #[resources] attribute should just be passed along
                            user_code.push(item.clone());
                        }
                    }
                }

                Item::ForeignMod(mod_) => {
                    // if !util::abi_is_c(&mod_.abi) {
                    //     return Err(parse::Error::new(
                    //         mod_.abi.extern_token.span(),
                    //         "this `extern` block must use the \"C\" abi",
                    //     ));
                    // }

                    for item in mod_.items {
                        if let ForeignItem::Fn(item) = item {
                            eprintln!("--- foreign Fn -- {}", item.sig.ident);
                            eprintln!("attr {:?}", item.attrs);
                            if settings.parse_extern_interrupt {
                                let (ident, extern_interrupt) = ExternInterrupt::parse(item)?;

                                let span = ident.span();
                                match extern_interrupts.entry(ident) {
                                    Entry::Occupied(..) => {
                                        return Err(parse::Error::new(
                                            span,
                                            "this extern interrupt is listed more than once",
                                        ));
                                    }

                                    Entry::Vacant(entry) => {
                                        entry.insert(extern_interrupt);
                                    }
                                }
                            } else {
                                return Err(parse::Error::new(
                                    item.sig.ident.span(),
                                    "this item must live outside the `#[app]` module",
                                ));
                            }
                        } else {
                            panic!("not fn");
                            // eprintln!("--- not fn ---");
                            // return Err(parse::Error::new(
                            //     item.span(),
                            //     "this item must live outside the `#[app]` module",
                            // ));
                        }
                    }
                }
                Item::Use(itemuse_) => {
                    // Store the user provided use-statements
                    user_imports.push(itemuse_.clone());
                }

                _ => {
                    eprintln!("-- not recognized -- {:?}", &item);
                    // Anything else within the module should not make any difference
                    user_code.push(item.clone());
                }
            }
        }

        Ok(App {
            args,

            name: input.ident,

            inits,
            idles,

            late_resources,
            resources,
            user_imports,
            user_code,
            hardware_tasks,
            software_tasks,

            extern_interrupts,

            _extensible: (),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{ast::AppArgs, ast::CustomArg};

    #[test]
    fn parse_app_args() {
        let s = "peripherals = true";

        let stream: proc_macro2::TokenStream = s.parse().unwrap();
        let result = AppArgs::parse(stream).unwrap();

        // Check map
        for (ident, value) in result.custom {
            match ident.to_string().as_ref() {
                "peripherals" => match value {
                    CustomArg::Bool(true) => {}
                    _ => panic!("Expected peripherals = true"),
                },
                _ => panic!("Unexpected identifier"),
            }
        }
    }
}
