use apollo_compiler::{Schema, validation::Valid};
use quote::{format_ident, quote};
use std::{collections::BTreeMap, fs, ops::Range, path::Path};
use syn::{FnArg, ImplItem, ImplItemFn, Item, Type, ext::IdentExt, spanned::Spanned};

use crate::{BuildError, codegen};

pub(crate) fn synchronize(schema: &Valid<Schema>, path: &Path) -> Result<(), BuildError> {
    let source_error = |source| BuildError::ResolverSource {
        path: path.to_owned(),
        source,
    };
    let existing = read_existing(path)?;
    let query = schema
        .schema_definition
        .query
        .as_ref()
        .and_then(|name| schema.get_object(name.as_str()))
        .ok_or_else(|| {
            source_error(syn::Error::new(
                proc_macro2::Span::call_site(),
                "A query root object is required",
            ))
        })?;
    let object_name = codegen::rust_name(query.name.as_str());
    // syn's identifier parser is edition-independent and accepts `gen`.
    let object = syn::parse_str::<syn::Ident>(&object_name)
        .ok()
        .filter(|_| object_name != "gen")
        .unwrap_or_else(|| format_ident!("r#{}", object_name));
    let resolver = format_ident!("{}Resolver", codegen::rust_name(query.name.as_str()));
    let methods = query
        .fields
        .iter()
        .map(|(name, field)| {
            let name = format_ident!("r#{}", codegen::rust_name(name.as_str()));
            let result = codegen::resolver_return_type(
                schema,
                query.name.as_str(),
                field.name.as_str(),
                &field.ty,
            )
            .map_err(BuildError::Codegen)?;
            Ok(syn::parse_quote! {
                async fn #name(
                    &self,
                    _context: &C,
                    _args: crate::generated::types::#object::#name::Args,
                ) -> ::core::result::Result<#result, ::necrassrs::ResolverError> {
                    ::core::unimplemented!()
                }
            })
        })
        .collect::<Result<Vec<ImplItemFn>, BuildError>>()?;

    let updated = match existing.as_deref() {
        None => format!(
            "#[allow(non_camel_case_types)]\npub struct {object};\n\n\
             #[allow(non_snake_case)]\n\
             impl<C: ::core::marker::Sync> crate::generated::resolvers::{resolver}<C> for self::{object} {{\n{}\n}}\n",
            methods
                .iter()
                .map(render_method)
                .collect::<Vec<_>>()
                .join("\n\n"),
        ),
        Some(source) => {
            update_existing(source, &object, &resolver, &methods).map_err(source_error)?
        }
    };
    syn::parse_file(&updated).map_err(source_error)?;

    if existing.as_deref() != Some(updated.as_str()) {
        if read_existing(path)? != existing {
            return Err(source_error(syn::Error::new(
                proc_macro2::Span::call_site(),
                "Resolver source changed during synchronization",
            )));
        }
        // ponytail: direct writes are not atomic; use atomic replacement if required later.
        fs::write(path, updated).map_err(|source| BuildError::Io {
            path: path.to_owned(),
            source,
        })?;
    }
    println!("cargo::rerun-if-changed={}", path.display());
    Ok(())
}

fn read_existing(path: &Path) -> Result<Option<String>, BuildError> {
    let read = || -> std::io::Result<Option<String>> {
        if let Some(parent) = path.parent()
            && !fs::symlink_metadata(parent)?.file_type().is_dir()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Resolver parent must be a directory, not a symbolic link",
            ));
        }
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
            Ok(metadata) if !metadata.file_type().is_file() => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Resolver destination must be a regular file, not a symbolic link",
            )),
            Ok(_) => fs::read_to_string(path).map(Some),
        }
    };
    read().map_err(|source| BuildError::Io {
        path: path.to_owned(),
        source,
    })
}

fn update_existing(
    source: &str,
    object: &syn::Ident,
    resolver: &syn::Ident,
    methods: &[ImplItemFn],
) -> syn::Result<String> {
    let ast = syn::parse_file(source)?;
    // parse_file removes these prefixes before assigning token byte ranges.
    let source_offset = if source.starts_with('\u{feff}') { 3 } else { 0 }
        + ast.shebang.as_ref().map_or(0, String::len);
    let mut implementations = ast.items.iter().filter_map(|item| {
        let Item::Impl(item) = item else { return None };
        let (_, trait_path, _) = item.trait_.as_ref()?;
        let Type::Path(self_type) = item.self_ty.as_ref() else {
            return None;
        };
        let expected = [
            "crate".to_owned(),
            "generated".to_owned(),
            "resolvers".to_owned(),
            resolver.to_string(),
        ];
        let matches_trait = trait_path
            .segments
            .iter()
            .map(|segment| segment.ident.unraw().to_string())
            .eq(expected);
        let self_path = &self_type.path;
        let matches_self = self_type.qself.is_none()
            && self_path.leading_colon.is_none()
            && (self_path.segments.len() == 1
                || (self_path.segments.len() == 2 && self_path.segments[0].ident == "self"))
            && self_path
                .segments
                .iter()
                .all(|segment| segment.arguments.is_none())
            && self_path
                .segments
                .last()
                .is_some_and(|segment| segment.ident.unraw() == object.unraw());
        (matches_trait && matches_self).then_some(item)
    });
    let implementation = implementations.next().ok_or_else(|| syn::Error::new(
        object.span(),
        "Expected one explicit crate::generated::resolvers implementation for the query root; aliases are not resolved",
    ))?;
    if implementations.next().is_some() {
        return Err(syn::Error::new_spanned(
            implementation,
            "Multiple query resolver implementations are ambiguous",
        ));
    }
    let context = implementation
        .trait_
        .as_ref()
        .and_then(|(_, path, _)| path.segments.last())
        .and_then(|segment| match &segment.arguments {
            syn::PathArguments::AngleBracketed(arguments) if arguments.args.len() == 1 => {
                arguments.args.first()
            }
            _ => None,
        })
        .and_then(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .ok_or_else(|| {
            syn::Error::new_spanned(implementation, "Expected one resolver Context type")
        })?;

    let mut desired: BTreeMap<_, _> = methods
        .iter()
        .map(|method| (method.sig.ident.unraw().to_string(), method))
        .collect();
    let mut edits = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for item in &implementation.items {
        let ImplItem::Fn(method) = item else { continue };
        let name = method.sig.ident.unraw().to_string();
        if !seen.insert(name.clone()) {
            return Err(syn::Error::new_spanned(
                method,
                "Duplicate resolver methods are ambiguous",
            ));
        }
        if desired.remove(&name).is_none() {
            edits.push(Edit {
                range: method.span().byte_range(),
                replacement: String::new(),
            });
            continue;
        }
        if method.sig.asyncness.is_none() || method.sig.inputs.len() != 3 {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "Expected an async resolver with self, Context, and Args parameters",
            ));
        }
        if !matches!(method.sig.inputs.last(), Some(FnArg::Typed(_))) {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "Expected a typed Args parameter",
            ));
        }
        // In the String!-only contract, retained fields keep the same return type
        // and Args path. Argument changes update Args in OUT_DIR, not this signature.
        // Preserve user spelling, aliases, and comments instead of normalizing them.
    }

    let added = desired
        .into_values()
        .map(|method| {
            let mut method = method.clone();
            if let Some(FnArg::Typed(argument)) = method.sig.inputs.iter_mut().nth(1) {
                *argument.ty = syn::parse_quote!(&#context);
            }
            render_method(&method)
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    if !added.is_empty() {
        let offset = implementation.brace_token.span.close().byte_range().start;
        edits.push(Edit {
            range: offset..offset,
            replacement: format!("\n{added}\n"),
        });
    }

    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let mut updated = source.to_owned();
    let mut next_start = source.len();
    for mut edit in edits {
        edit.range.start += source_offset;
        edit.range.end += source_offset;
        if edit.range.end > next_start || source.get(edit.range.clone()).is_none() {
            return Err(syn::Error::new_spanned(
                implementation,
                "Invalid or overlapping source edits",
            ));
        }
        next_start = edit.range.start;
        updated.replace_range(edit.range, &edit.replacement);
    }
    Ok(updated)
}

struct Edit {
    range: Range<usize>,
    replacement: String,
}

fn render_method(method: &ImplItemFn) -> String {
    let signature = &method.sig;
    format!(
        "    {} {{\n        ::core::unimplemented!()\n    }}",
        quote!(#signature)
    )
}
