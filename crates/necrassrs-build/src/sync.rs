use apollo_compiler::{Schema, ast::Type as GraphqlType, schema::ExtendedType, validation::Valid};
use quote::{ToTokens, format_ident, quote};
use std::{collections::BTreeMap, fs, ops::Range, path::Path};
use syn::{GenericArgument, Item, PathArguments, Type, ext::IdentExt, spanned::Spanned};

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
    let object_names = codegen::reachable_object_names(schema, query.name.as_str());
    let objects = object_names
        .iter()
        .map(|name| rust_ident(name))
        .collect::<Vec<_>>();
    let mut field_resolvers = query
        .fields
        .iter()
        .map(|(name, field)| {
            field_resolver(
                schema,
                query.name.as_str(),
                name.as_str(),
                &field.ty,
                query.name.as_str(),
            )
        })
        .collect::<Result<Vec<_>, BuildError>>()?;
    for field_resolver in &mut field_resolvers {
        field_resolver.receiver = syn::parse_quote!(self::#object);
    }
    for object_name in &object_names {
        let object_type = schema
            .get_object(object_name)
            .expect("reachable Object type must exist");
        field_resolvers.extend(
            object_type
                .fields
                .iter()
                .map(|(name, field)| {
                    field_resolver(
                        schema,
                        object_name,
                        name.as_str(),
                        &field.ty,
                        query.name.as_str(),
                    )
                })
                .collect::<Result<Vec<_>, BuildError>>()?,
        );
    }

    let updated = match existing
        .as_deref()
        .filter(|source| !source.trim().is_empty())
    {
        None => {
            let generics: syn::Generics = syn::parse_quote!(<C: ::core::marker::Sync>);
            let context: Type = syn::parse_quote!(C);
            let object_declarations = objects
                .iter()
                .map(|object| {
                    format!(
                        "#[allow(non_camel_case_types)]\n#[derive(Clone, Copy)]\npub struct {object};"
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let field_implementations = field_resolvers
                .iter()
                .map(|resolver| render_field_resolver(resolver, &generics, &context))
                .collect::<Vec<_>>()
                .join("\n\n");
            format!(
                "#[allow(non_camel_case_types)]\npub struct {object};\n\n\
                 {object_declarations}\n\n\
             #[allow(non_snake_case)]\n\
             impl<C: ::core::marker::Sync> crate::generated::resolvers::{resolver}<C> for self::{object} {{}}\n",
            ) + if field_implementations.is_empty() {
                ""
            } else {
                "\n"
            } + &field_implementations
                + if field_implementations.is_empty() {
                    ""
                } else {
                    "\n"
                }
        }
        Some(source) => update_existing(source, &object, &resolver, &objects, &field_resolvers)
            .map_err(source_error)?,
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
    objects: &[syn::Ident],
    field_resolvers: &[FieldResolver],
) -> syn::Result<String> {
    let ast = syn::parse_file(source)?;
    // parse_file removes these prefixes before assigning token byte ranges.
    let source_offset = if source.starts_with('\u{feff}') { 3 } else { 0 }
        + ast.shebang.as_ref().map_or(0, String::len);
    let implementation = query_resolver_implementation(&ast, object, resolver)?;
    let context = resolver_context(implementation)?;
    let generics = &implementation.generics;
    if !implementation.items.is_empty() {
        return Err(syn::Error::new_spanned(
            &implementation.items[0],
            "Expected an empty query root resolver marker; grouped resolver methods are not supported",
        ));
    }
    let mut edits = Vec::new();
    let root_receiver = implementation.self_ty.as_ref();
    let mut field_resolvers = field_resolvers.to_vec();
    for field_resolver in &mut field_resolvers {
        if field_resolver.object.unraw() == object.unraw() {
            field_resolver.receiver = root_receiver.clone();
        }
    }
    let desired_fields = reconcile_field_resolvers(&ast, &field_resolvers, &mut edits)?;

    let defined_types = ast
        .items
        .iter()
        .filter_map(item_type_name)
        .collect::<std::collections::BTreeSet<_>>();
    let declarations = objects
        .iter()
        .filter(|object| !defined_types.contains(&object.unraw().to_string()))
        .map(|object| {
            format!("#[allow(non_camel_case_types)]\n#[derive(Clone, Copy)]\npub struct {object};")
        });
    let additions = declarations
        .chain(
            desired_fields
                .into_values()
                .map(|resolver| render_field_resolver(resolver, generics, context)),
        )
        .collect::<Vec<_>>()
        .join("\n\n");
    if !additions.is_empty() {
        edits.push(Edit {
            range: source.len() - source_offset..source.len() - source_offset,
            replacement: format!("\n{additions}\n"),
        });
    }

    apply_edits(source, source_offset, implementation, edits)
}

fn query_resolver_implementation<'a>(
    ast: &'a syn::File,
    object: &syn::Ident,
    resolver: &syn::Ident,
) -> syn::Result<&'a syn::ItemImpl> {
    let mut implementations = ast
        .items
        .iter()
        .filter_map(|item| matching_query_resolver(item, resolver));
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
    Ok(implementation)
}

fn matching_query_resolver<'a>(item: &'a Item, resolver: &syn::Ident) -> Option<&'a syn::ItemImpl> {
    let Item::Impl(item) = item else { return None };
    let (_, trait_path, _) = item.trait_.as_ref()?;
    let expected = [
        "crate".to_owned(),
        "generated".to_owned(),
        "resolvers".to_owned(),
        resolver.to_string(),
    ];
    trait_path
        .segments
        .iter()
        .map(|segment| segment.ident.unraw().to_string())
        .eq(expected)
        .then_some(item)
}

fn resolver_context(implementation: &syn::ItemImpl) -> syn::Result<&Type> {
    implementation
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
        })
}

fn reconcile_field_resolvers<'a>(
    ast: &syn::File,
    field_resolvers: &'a [FieldResolver],
    edits: &mut Vec<Edit>,
) -> syn::Result<BTreeMap<(String, String), &'a FieldResolver>> {
    let mut desired_fields = field_resolvers
        .iter()
        .map(|resolver| (resolver.coordinate(), resolver))
        .collect::<BTreeMap<_, _>>();
    let mut seen_fields = std::collections::BTreeSet::new();
    for item in ast.items.iter().filter_map(|item| match item {
        Item::Impl(item) => Some(item),
        _ => None,
    }) {
        let Some(coordinate) = field_resolver_coordinate(item) else {
            continue;
        };
        if !seen_fields.insert(coordinate.clone()) {
            return Err(syn::Error::new_spanned(
                item,
                "Duplicate field resolver implementations are ambiguous",
            ));
        }
        if let Some(resolver) = desired_fields.remove(&coordinate) {
            reconcile_field_resolver(item, resolver, edits)?;
        } else {
            edits.push(Edit {
                range: item.span().byte_range(),
                replacement: String::new(),
            });
        }
    }
    Ok(desired_fields)
}

fn reconcile_field_resolver(
    implementation: &syn::ItemImpl,
    resolver: &FieldResolver,
    edits: &mut Vec<Edit>,
) -> syn::Result<()> {
    let current_receiver = &implementation.self_ty;
    let desired_receiver = &resolver.receiver;
    if quote!(#current_receiver).to_string() != quote!(#desired_receiver).to_string() {
        let replacement = &resolver.receiver;
        edits.push(Edit {
            range: implementation.self_ty.span().byte_range(),
            replacement: quote!(#replacement).to_string(),
        });
    }
    let mut outputs = implementation.items.iter().filter_map(|item| match item {
        syn::ImplItem::Type(item) if item.ident == "Output" => Some(item),
        _ => None,
    });
    let output = outputs.next().ok_or_else(|| {
        syn::Error::new_spanned(
            implementation,
            "Expected one Output type in a field resolver implementation",
        )
    })?;
    if outputs.next().is_some() {
        return Err(syn::Error::new_spanned(
            implementation,
            "Expected one Output type in a field resolver implementation",
        ));
    }
    if !equivalent_output_type(&output.ty, &resolver.output) {
        let replacement = &resolver.output;
        edits.push(Edit {
            range: output.ty.span().byte_range(),
            replacement: quote!(#replacement).to_string(),
        });
    }
    Ok(())
}

fn equivalent_output_type(left: &Type, right: &Type) -> bool {
    match (left, right) {
        (Type::Group(left), _) => equivalent_output_type(&left.elem, right),
        (_, Type::Group(right)) => equivalent_output_type(left, &right.elem),
        (Type::Paren(left), _) => equivalent_output_type(&left.elem, right),
        (_, Type::Paren(right)) => equivalent_output_type(left, &right.elem),
        (Type::Path(left), Type::Path(right)) if left.qself.is_none() && right.qself.is_none() => {
            let Some(left) = left.path.segments.last() else {
                return false;
            };
            let Some(right) = right.path.segments.last() else {
                return false;
            };
            left.ident.unraw() == right.ident.unraw()
                && equivalent_path_arguments(&left.arguments, &right.arguments)
        }
        _ => quote!(#left).to_string() == quote!(#right).to_string(),
    }
}

fn equivalent_path_arguments(left: &PathArguments, right: &PathArguments) -> bool {
    match (left, right) {
        (PathArguments::None, PathArguments::None) => true,
        (PathArguments::AngleBracketed(left), PathArguments::AngleBracketed(right)) => {
            left.args.len() == right.args.len()
                && left
                    .args
                    .iter()
                    .zip(&right.args)
                    .all(|(left, right)| match (left, right) {
                        (GenericArgument::Type(left), GenericArgument::Type(right)) => {
                            equivalent_output_type(left, right)
                        }
                        _ => quote!(#left).to_string() == quote!(#right).to_string(),
                    })
        }
        _ => false,
    }
}

fn apply_edits(
    source: &str,
    source_offset: usize,
    implementation: &syn::ItemImpl,
    mut edits: Vec<Edit>,
) -> syn::Result<String> {
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

#[derive(Clone)]
struct FieldResolver {
    object: syn::Ident,
    field: syn::Ident,
    receiver: Type,
    output: Type,
}

impl FieldResolver {
    fn coordinate(&self) -> (String, String) {
        (
            self.object.unraw().to_string(),
            self.field.unraw().to_string(),
        )
    }
}

fn field_resolver(
    schema: &Schema,
    object_name: &str,
    field_name: &str,
    ty: &GraphqlType,
    query_name: &str,
) -> Result<FieldResolver, BuildError> {
    let output = resolver_output_type(schema, object_name, field_name, ty, query_name)
        .map_err(BuildError::Codegen)?;
    let object = rust_ident(object_name);
    Ok(FieldResolver {
        receiver: syn::parse_quote!(self::#object),
        object,
        field: rust_ident(field_name),
        output: syn::parse2(output).expect("generated resolver output types must parse"),
    })
}

fn resolver_output_type(
    schema: &Schema,
    object_name: &str,
    field_name: &str,
    ty: &GraphqlType,
    query_name: &str,
) -> Result<proc_macro2::TokenStream, codegen::CodegenError> {
    if codegen::composite_output_name(schema, ty).is_none() {
        return codegen::resolver_return_type(
            schema,
            object_name,
            field_name,
            ty,
            &quote! { crate::generated::types },
        );
    }
    match composite_resolver_output_type(schema, ty, query_name) {
        Some(output) => Ok(output),
        None => codegen::resolver_return_type(
            schema,
            object_name,
            field_name,
            ty,
            &quote! { crate::generated::types },
        ),
    }
}

fn composite_resolver_output_type(
    schema: &Schema,
    ty: &GraphqlType,
    query_name: &str,
) -> Option<proc_macro2::TokenStream> {
    match ty {
        GraphqlType::NonNullNamed(name) => {
            named_composite_resolver_output_type(schema, name, query_name)
        }
        GraphqlType::Named(name) => {
            let output = named_composite_resolver_output_type(schema, name, query_name)?;
            Some(quote! { ::core::option::Option<#output> })
        }
        GraphqlType::NonNullList(item) => {
            let item = composite_resolver_output_type(schema, item, query_name)?;
            Some(quote! { ::std::vec::Vec<#item> })
        }
        GraphqlType::List(item) => {
            let item = composite_resolver_output_type(schema, item, query_name)?;
            Some(quote! { ::core::option::Option<::std::vec::Vec<#item>> })
        }
    }
}

fn named_composite_resolver_output_type(
    schema: &Schema,
    name: &apollo_compiler::Name,
    query_name: &str,
) -> Option<proc_macro2::TokenStream> {
    match schema.types.get(name)? {
        ExtendedType::Object(_) if name.as_str() != query_name => {
            let object = rust_ident(name.as_str());
            Some(quote! { self::#object })
        }
        ExtendedType::Interface(_) | ExtendedType::Union(_) => {
            let abstract_type = rust_ident(name.as_str());
            let members = codegen::abstract_member_names(schema, name)?
                .into_iter()
                .map(|member| {
                    (member != query_name).then(|| {
                        let member = rust_ident(member);
                        quote! { self::#member }
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(quote! { crate::generated::types::#abstract_type<#(#members),*> })
        }
        _ => None,
    }
}

fn rust_ident(name: &str) -> syn::Ident {
    format_ident!("r#{}", codegen::rust_name(name))
}

fn render_field_resolver(
    resolver: &FieldResolver,
    generics: &syn::Generics,
    context: &Type,
) -> String {
    let object = &resolver.object;
    let field = &resolver.field;
    let receiver = &resolver.receiver;
    let output = &resolver.output;
    let mut impl_generics = proc_macro2::TokenStream::new();
    generics.to_tokens(&mut impl_generics);
    let where_clause = &generics.where_clause;
    format!(
        "impl{} ::necrassrs::Resolver<crate::generated::fields::{object}::{field}, {}> for {} {}{{\n\
         \x20   type Output = {};\n\n\
         \x20   async fn resolve(\n\
         \x20       &self,\n\
         \x20       _context: &{},\n\
         \x20       _args: crate::generated::types::{object}::{field}::Args,\n\
         \x20   ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {{\n\
         \x20       ::core::unimplemented!()\n\
         \x20   }}\n\
         }}",
        impl_generics,
        quote!(#context),
        quote!(#receiver),
        quote!(#where_clause),
        quote!(#output),
        quote!(#context),
    )
}

fn field_resolver_coordinate(implementation: &syn::ItemImpl) -> Option<(String, String)> {
    let (_, trait_path, _) = implementation.trait_.as_ref()?;
    let expected = ["necrassrs", "Resolver"];
    if !trait_path
        .segments
        .iter()
        .map(|segment| segment.ident.unraw().to_string())
        .eq(expected.map(str::to_owned))
    {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &trait_path.segments.last()?.arguments else {
        return None;
    };
    let GenericArgument::Type(Type::Path(field_type)) = arguments.args.first()? else {
        return None;
    };
    let field_path = field_type
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.unraw().to_string())
        .collect::<Vec<_>>();
    let [crate_name, generated, fields, object, field] = field_path.as_slice() else {
        return None;
    };
    if [crate_name.as_str(), generated, fields] != ["crate", "generated", "fields"] {
        return None;
    }
    Some((object.clone(), field.clone()))
}

fn item_type_name(item: &Item) -> Option<String> {
    // shortcut: imported and cross-file object types are discovered by the #38 module AST work.
    let ident = match item {
        Item::Enum(item) => &item.ident,
        Item::Struct(item) => &item.ident,
        Item::Type(item) => &item.ident,
        Item::Union(item) => &item.ident,
        _ => return None,
    };
    Some(ident.unraw().to_string())
}

struct Edit {
    range: Range<usize>,
    replacement: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ResolverFile {
        directory: std::path::PathBuf,
        path: std::path::PathBuf,
    }

    impl ResolverFile {
        fn new() -> Self {
            static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "necrassrs-sync-unit-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&directory).unwrap();
            let path = directory.join("resolvers.rs");
            Self { directory, path }
        }

        fn synchronize(&self, sdl: &str) -> Result<(), BuildError> {
            let schema = Schema::parse_and_validate(sdl, "schema.graphql").unwrap();
            super::synchronize(&schema, &self.path)
        }

        fn read(&self) -> String {
            fs::read_to_string(&self.path).unwrap()
        }
    }

    impl Drop for ResolverFile {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    #[test]
    fn creates_resolvers_and_preserves_unchanged_source() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! }").unwrap();
        let source = file.read();
        let ast = syn::parse_file(&source).unwrap();
        assert!(matches!(&ast.items[0], Item::Struct(item) if item.ident == "Query"));
        assert!(source.contains("fields::r#Query::r#hello"));
        assert!(source.contains("async fn resolve"));
        assert!(source.contains("unimplemented"));

        file.synchronize("type Query { hello: String! }").unwrap();
        assert_eq!(file.read(), source);
    }

    #[test]
    fn whitespace_only_source_bootstraps_resolvers() {
        let file = ResolverFile::new();
        fs::write(&file.path, "  \n\t\n").unwrap();

        file.synchronize("type Query { hello: String! }").unwrap();

        let source = file.read();
        assert!(source.contains("QueryResolver<C> for self::Query {}"));
        assert!(source.contains("fields::r#Query::r#hello"));
    }

    #[test]
    fn custom_query_root_and_context_drive_query_field_resolvers() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! }").unwrap();
        let source = file
            .read()
            .replacen(
                "pub struct Query;",
                "pub struct AppContext;\npub struct AppQuery;",
                1,
            )
            .replace(
                "impl<C: ::core::marker::Sync> crate::generated::resolvers::QueryResolver<C> for self::Query {}",
                "impl crate::generated::resolvers::QueryResolver<AppContext> for self::AppQuery {}",
            )
            .replacen(
                "::core::unimplemented!()",
                "{ Ok(::std::string::String::from(\"retained\")) }",
                1,
            );
        syn::parse_file(&source).unwrap();
        fs::write(&file.path, source).unwrap();

        file.synchronize("type Query { hello: String! extra: String! }")
            .unwrap();

        let source = file.read();
        let ast = syn::parse_file(&source).unwrap();
        for field in ["hello", "extra"] {
            let implementation = ast
                .items
                .iter()
                .find_map(|item| match item {
                    Item::Impl(item)
                        if field_resolver_coordinate(item)
                            == Some(("Query".to_owned(), field.to_owned())) =>
                    {
                        Some(item)
                    }
                    _ => None,
                })
                .unwrap();
            let receiver = &implementation.self_ty;
            assert_eq!(quote!(#receiver).to_string(), "self :: AppQuery");
            let resolve = implementation
                .items
                .iter()
                .find_map(|item| match item {
                    syn::ImplItem::Fn(method) if method.sig.ident == "resolve" => Some(method),
                    _ => None,
                })
                .unwrap();
            if field == "hello" {
                assert!(
                    resolve
                        .block
                        .to_token_stream()
                        .to_string()
                        .contains("retained")
                );
            } else {
                assert!(
                    quote!(#resolve)
                        .to_string()
                        .contains("_context : & AppContext")
                );
            }
        }
        let marker = query_resolver_implementation(
            &ast,
            &syn::parse_quote!(Query),
            &syn::parse_quote!(QueryResolver),
        )
        .unwrap();
        let receiver = &marker.self_ty;
        let context = resolver_context(marker).unwrap();
        assert_eq!(quote!(#receiver).to_string(), "self :: AppQuery");
        assert_eq!(quote!(#context).to_string(), "AppContext");
    }

    #[test]
    fn adds_and_removes_field_resolvers_without_rewriting_retained_code() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! old: String! }")
            .unwrap();
        let source = file.read().replace(
            "::core::unimplemented!()",
            "{ /* retain this comment */ Ok(String::from(\"hello\")) }",
        );
        fs::write(&file.path, &source).unwrap();

        file.synchronize("type Query { hello: String! added: String! }")
            .unwrap();
        let updated = file.read();
        assert!(updated.contains("/* retain this comment */"));
        assert!(updated.contains("fields::r#Query::r#added"));
        assert!(!updated.contains("fields::r#Query::r#old"));
        syn::parse_file(&updated).unwrap();
    }

    #[test]
    fn source_prefixes_do_not_shift_edits() {
        for prefix in ["\u{feff}", "#!/usr/bin/env rust-script\n"] {
            let file = ResolverFile::new();
            file.synchronize("type Query { hello: String! }").unwrap();
            let source = format!("{prefix}{}", file.read());
            fs::write(&file.path, &source).unwrap();
            file.synchronize("type Query { hello: String! added: String! }")
                .unwrap();
            let updated = file.read();
            assert!(updated.starts_with(prefix));
            assert!(updated.contains("fields::r#Query::r#hello"));
            assert!(updated.contains("fields::r#Query::r#added"));
        }
    }

    #[test]
    fn added_field_resolver_uses_the_existing_context_type() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! }").unwrap();
        let source = file.read().replace(
            "QueryResolver<C> for self::Query",
            "QueryResolver<AppContext> for self::Query",
        );
        fs::write(&file.path, source).unwrap();

        file.synchronize("type Query { hello: String! added: String! }")
            .unwrap();
        let ast = syn::parse_file(&file.read()).unwrap();
        let implementation = ast
            .items
            .iter()
            .find_map(|item| match item {
                Item::Impl(item)
                    if field_resolver_coordinate(item)
                        == Some(("Query".to_owned(), "added".to_owned())) =>
                {
                    Some(item)
                }
                _ => None,
            })
            .unwrap();
        let added = implementation
            .items
            .iter()
            .find_map(|item| match item {
                syn::ImplItem::Fn(method) if method.sig.ident == "resolve" => Some(method),
                _ => None,
            })
            .unwrap();
        let context = &added.sig.inputs[1];
        assert_eq!(quote!(#context).to_string(), "_context : & AppContext");
    }

    #[test]
    fn updates_field_output_shapes_without_rewriting_the_body() {
        let file = ResolverFile::new();
        file.synchronize("type Query { value: String! }").unwrap();
        let source = file.read().replacen(
            "::core::unimplemented!()",
            "{ /* retain this body */ ::core::unimplemented!() }",
            1,
        );
        fs::write(&file.path, source).unwrap();

        for (sdl, expected) in [
            (
                "type Query { value: String }",
                ":: core :: option :: Option < :: std :: string :: String >",
            ),
            (
                "type Query { value: [String!]! }",
                ":: std :: vec :: Vec < :: std :: string :: String >",
            ),
            (
                "type Query { value: User! } type User { name: String! }",
                "self :: r#User",
            ),
            ("type Query { value: Int! }", ":: core :: primitive :: i32"),
        ] {
            file.synchronize(sdl).unwrap();
            let source = file.read();
            let ast = syn::parse_file(&source).unwrap();
            let implementation = ast
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Impl(item)
                        if field_resolver_coordinate(item)
                            == Some(("Query".to_owned(), "value".to_owned())) =>
                    {
                        Some(item)
                    }
                    _ => None,
                })
                .next()
                .unwrap();
            let output = implementation
                .items
                .iter()
                .find_map(|item| match item {
                    syn::ImplItem::Type(item) if item.ident == "Output" => Some(&item.ty),
                    _ => None,
                })
                .unwrap();

            assert_eq!(quote!(#output).to_string(), expected, "{sdl}");
            assert!(source.contains("/* retain this body */"), "{sdl}");
        }
    }

    #[test]
    fn rejects_ambiguous_or_malformed_implementations_without_writing() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! }").unwrap();
        let source = file.read();
        let cases = [
            (
                source.replace("crate::generated::resolvers", "resolvers"),
                "Expected one explicit",
            ),
            (
                format!("{source}\n{}", &source[source.find("impl<").unwrap()..]),
                "Multiple query",
            ),
            (
                source.replace("QueryResolver<C>", "QueryResolver"),
                "Expected one resolver Context",
            ),
            (
                source.replace(
                    "for self::Query {}",
                    "for self::Query { async fn hello(&self) {} }",
                ),
                "Expected an empty query root resolver marker",
            ),
            (
                format!("{source}\n{}", &source[source.rfind("impl<").unwrap()..]),
                "Duplicate field resolver",
            ),
            (
                source.replacen("type Output", "type Result", 1),
                "Expected one Output type",
            ),
            ("not Rust source".to_owned(), "expected"),
        ];
        for (index, (input, expected)) in cases.into_iter().enumerate() {
            fs::write(&file.path, &input).unwrap();
            let error = file
                .synchronize("type Query { hello: String! }")
                .expect_err(&format!("case {index} unexpectedly succeeded"));
            assert!(
                error.to_string().contains(expected),
                "case {index}: {error}"
            );
            assert_eq!(file.read(), input);
        }
    }

    #[test]
    fn rejects_unsupported_return_type_before_creating_source() {
        let file = ResolverFile::new();
        let error = file
            .synchronize("scalar Timestamp\ntype Query { count: Timestamp! }")
            .unwrap_err();
        assert!(matches!(error, BuildError::Codegen(_)));
        assert!(!file.path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_destination_and_parent() {
        use std::os::unix::fs::symlink;

        let file = ResolverFile::new();
        let target = file.directory.join("target.rs");
        fs::write(&target, "user code").unwrap();
        symlink(&target, &file.path).unwrap();
        assert!(matches!(
            file.synchronize("type Query { hello: String! }"),
            Err(BuildError::Io { .. })
        ));
        assert_eq!(fs::read_to_string(target).unwrap(), "user code");

        let parent = file.directory.join("linked");
        symlink(&file.directory, &parent).unwrap();
        let linked = parent.join("new.rs");
        let schema =
            Schema::parse_and_validate("type Query { hello: String! }", "schema.graphql").unwrap();
        assert!(matches!(
            super::synchronize(&schema, &linked),
            Err(BuildError::Io { .. })
        ));
        assert!(!file.directory.join("new.rs").exists());
    }
}
