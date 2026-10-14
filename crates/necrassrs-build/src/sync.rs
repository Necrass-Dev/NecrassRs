use apollo_compiler::{Schema, ast::Type as GraphqlType, schema::ExtendedType, validation::Valid};
use quote::{format_ident, quote};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    ops::Range,
    path::{Path, PathBuf},
};
use syn::{
    GenericArgument, Item, PathArguments, Type, ext::IdentExt, spanned::Spanned, visit::Visit,
    visit_mut::VisitMut,
};

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
    let roots = std::iter::once(query)
        .chain(
            schema
                .schema_definition
                .mutation
                .as_ref()
                .and_then(|name| schema.get_object(name.as_str())),
        )
        .collect::<Vec<_>>();
    let root_types = roots
        .iter()
        .map(|root| root.name.as_str())
        .collect::<Vec<_>>();
    let root_objects = roots
        .iter()
        .map(|root| rust_ident(root.name.as_str()))
        .collect::<Vec<_>>();
    let root_resolvers = roots
        .iter()
        .map(|root| format_ident!("{}Resolver", codegen::rust_name(root.name.as_str())))
        .collect::<Vec<_>>();
    let object_names = codegen::reachable_object_names(schema, &root_types);
    let objects = object_names
        .iter()
        .map(|name| rust_ident(name))
        .collect::<Vec<_>>();
    let mut field_resolvers = Vec::new();
    for root in &roots {
        field_resolvers.extend(
            root.fields
                .iter()
                .map(|(name, field)| {
                    field_resolver(
                        schema,
                        root.name.as_str(),
                        name.as_str(),
                        field
                            .arguments
                            .iter()
                            .map(|argument| codegen::rust_name(argument.name.as_str()))
                            .collect(),
                        &field.ty,
                        &root_types,
                    )
                })
                .collect::<Result<Vec<_>, BuildError>>()?,
        );
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
                        field
                            .arguments
                            .iter()
                            .map(|argument| codegen::rust_name(argument.name.as_str()))
                            .collect(),
                        &field.ty,
                        &root_types,
                    )
                })
                .collect::<Result<Vec<_>, BuildError>>()?,
        );
    }

    let mut writes = Vec::new();
    let mut watched_paths = BTreeSet::from([path.to_owned()]);
    let updated = match existing
        .as_deref()
        .filter(|source| !source.trim().is_empty())
    {
        None => {
            let mut scope = RenderScope::default();
            scope.names.extend(
                objects
                    .iter()
                    .chain(&root_objects)
                    .map(|name| name.unraw().to_string()),
            );
            let sync = scope.standard("Sync");
            let generics: syn::Generics = syn::parse_str(&format!("<C: {sync}>")).unwrap();
            let context: Type = syn::parse_quote!(C);
            let root_declarations = root_objects
                .iter()
                .map(|object| format!("#[allow(non_camel_case_types)]\npub struct {object};"))
                .collect::<Vec<_>>()
                .join("\n\n");
            let object_declarations = objects
                .iter()
                .map(|object| {
                    format!(
                        "#[allow(non_camel_case_types)]\n#[derive(Clone, Copy)]\npub struct {object};"
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let root_implementations = root_objects
                .iter()
                .zip(&root_resolvers)
                .map(|(object, resolver)| {
                    format!(
                        "#[allow(non_snake_case)]\nimpl<C: {sync}> crate::generated::resolvers::{resolver}<C> for self::{object} {{}}"
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let field_implementations = field_resolvers
                .iter()
                .map(|resolver| render_field_resolver(resolver, &generics, &context, &scope))
                .collect::<Vec<_>>()
                .join("\n\n");
            format!(
                "{root_declarations}\n\n\
                 {object_declarations}\n\n\
                 {root_implementations}\n",
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
        Some(source) => {
            let external = reconcile_external_modules(path, source, &field_resolvers)?;
            watched_paths.extend(external.paths);
            writes.extend(external.writes);
            update_existing(
                path,
                source,
                &root_objects,
                &root_resolvers,
                &objects,
                &field_resolvers,
                &external.fields,
            )
            .map_err(source_error)?
        }
    };
    syn::parse_file(&updated).map_err(source_error)?;

    if existing.as_deref() != Some(updated.as_str()) {
        writes.push(PlannedWrite {
            path: path.to_owned(),
            original: existing,
            updated,
        });
    }
    for write in &writes {
        if read_existing(&write.path)? != write.original {
            return Err(BuildError::ResolverSource {
                path: write.path.clone(),
                source: syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "Resolver source changed during synchronization",
                ),
            });
        }
    }
    commit_writes(writes)?;
    for path in watched_paths {
        println!("cargo::rerun-if-changed={}", path.display());
    }
    Ok(())
}

struct ExternalReconciliation {
    fields: BTreeMap<(String, String), PathBuf>,
    paths: BTreeSet<PathBuf>,
    writes: Vec<PlannedWrite>,
}

struct PlannedWrite {
    path: PathBuf,
    original: Option<String>,
    updated: String,
}

fn commit_writes(writes: Vec<PlannedWrite>) -> Result<(), BuildError> {
    commit_writes_with(writes, replace_file, |path| fs::remove_file(path))
}

fn replace_file(path: &Path, source: &str) -> std::io::Result<()> {
    replace_file_with(
        path,
        source,
        |_, staged, source| {
            match fs::metadata(path) {
                Ok(metadata) => staged.set_permissions(metadata.permissions())?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            std::io::Write::write_all(staged, source.as_bytes())
        },
        |staged, target| fs::rename(staged, target),
    )
}

fn replace_file_with(
    path: &Path,
    source: &str,
    write_staged: impl FnOnce(&Path, &mut fs::File, &str) -> std::io::Result<()>,
    replace: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    static NEXT_STAGING_FILE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut attempt = 0;
    let (staged_path, mut staged) = loop {
        let sequence = NEXT_STAGING_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let staged_path = parent.join(format!(".necrassrs-{}-{sequence}.tmp", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged_path)
        {
            Ok(staged) => break (staged_path, staged),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempt < 127 => {
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    };

    let result = write_staged(&staged_path, &mut staged, source);
    drop(staged);
    let result = result.and_then(|()| replace(&staged_path, path));
    if let Err(error) = result {
        return match fs::remove_file(&staged_path) {
            Ok(()) => Err(error),
            Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => Err(error),
            Err(cleanup) => Err(std::io::Error::new(
                error.kind(),
                format!(
                    "{error}; could not remove staging file {}: {cleanup}",
                    staged_path.display()
                ),
            )),
        };
    }
    Ok(())
}

fn commit_writes_with(
    writes: Vec<PlannedWrite>,
    mut write_file: impl FnMut(&Path, &str) -> std::io::Result<()>,
    mut remove_file: impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<(), BuildError> {
    for (index, write) in writes.iter().enumerate() {
        let failure = match read_existing(&write.path) {
            Ok(current) if current != write.original => Some(std::io::Error::other(
                "Resolver source changed during synchronization",
            )),
            Ok(_) => write_file(&write.path, &write.updated).err(),
            Err(error) => Some(std::io::Error::other(error.to_string())),
        };
        let Some(source) = failure else {
            continue;
        };
        return rollback_writes(
            &writes[..index],
            &write.path,
            source,
            &mut write_file,
            &mut remove_file,
        );
    }
    Ok(())
}

fn rollback_writes(
    writes: &[PlannedWrite],
    failed_path: &Path,
    commit_error: std::io::Error,
    write_file: &mut impl FnMut(&Path, &str) -> std::io::Result<()>,
    remove_file: &mut impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<(), BuildError> {
    let mut rollback_failure = None;
    for write in writes.iter().rev() {
        let result = match read_existing(&write.path) {
            Ok(Some(current)) if current == write.updated => match &write.original {
                Some(original) => write_file(&write.path, original),
                None => match remove_file(&write.path) {
                    Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    result => result,
                },
            },
            Ok(_) => Err(std::io::Error::other(
                "Resolver source changed during rollback",
            )),
            Err(error) => Err(std::io::Error::other(error.to_string())),
        };
        if let Err(source) = result
            && rollback_failure.is_none()
        {
            rollback_failure = Some((write.path.clone(), source));
        }
    }
    if let Some((rollback_path, rollback_error)) = rollback_failure {
        return Err(BuildError::Io {
            path: rollback_path.clone(),
            source: std::io::Error::other(format!(
                "Commit failed for {}: {commit_error}; rollback failed for {}: {rollback_error}",
                failed_path.display(),
                rollback_path.display()
            )),
        });
    }
    Err(BuildError::Io {
        path: failed_path.to_owned(),
        source: commit_error,
    })
}

fn reconcile_external_modules(
    entry_path: &Path,
    entry_source: &str,
    field_resolvers: &[FieldResolver],
) -> Result<ExternalReconciliation, BuildError> {
    let entry = syn::parse_file(entry_source).map_err(|source| BuildError::ResolverSource {
        path: entry_path.to_owned(),
        source,
    })?;
    let desired = field_resolvers
        .iter()
        .map(|resolver| (resolver.coordinate(), resolver))
        .collect::<BTreeMap<_, _>>();
    let mut fields = BTreeMap::new();
    let mut paths = BTreeSet::new();
    let mut writes = Vec::new();

    reconcile_external_children(
        entry_path,
        &entry,
        &desired,
        &mut fields,
        &mut paths,
        &mut writes,
    )?;

    Ok(ExternalReconciliation {
        fields,
        paths,
        writes,
    })
}

fn reconcile_external_children(
    parent_path: &Path,
    parent: &syn::File,
    desired: &BTreeMap<(String, String), &FieldResolver>,
    fields: &mut BTreeMap<(String, String), PathBuf>,
    paths: &mut BTreeSet<PathBuf>,
    writes: &mut Vec<PlannedWrite>,
) -> Result<(), BuildError> {
    let module_directory = if parent_path.ends_with("mod.rs") {
        parent_path.parent().unwrap_or(Path::new("")).to_owned()
    } else {
        parent_path.with_extension("")
    };
    reconcile_external_children_in(
        parent_path,
        &parent.items,
        &module_directory,
        desired,
        fields,
        paths,
        writes,
    )
}

fn reconcile_external_children_in(
    source_path: &Path,
    items: &[Item],
    module_directory: &Path,
    desired: &BTreeMap<(String, String), &FieldResolver>,
    fields: &mut BTreeMap<(String, String), PathBuf>,
    paths: &mut BTreeSet<PathBuf>,
    writes: &mut Vec<PlannedWrite>,
) -> Result<(), BuildError> {
    if let Some(item) = items.iter().find_map(|item| match item {
        Item::Macro(item) if item.ident.is_none() => Some(item),
        _ => None,
    }) {
        let path = &item.mac.path;
        return Err(BuildError::ResolverSource {
            path: source_path.to_owned(),
            source: syn::Error::new_spanned(
                item,
                format!(
                    "Resolver module discovery does not support {}!",
                    quote!(#path)
                ),
            ),
        });
    }
    for module in items.iter().filter_map(|item| match item {
        Item::Mod(module) => Some(module),
        _ => None,
    }) {
        if is_cfg_test_module(module) {
            continue;
        }
        let unsupported_attribute = module.attrs.iter().find_map(|attribute| {
            if attribute.path().is_ident("path") {
                Some("path")
            } else if attribute.path().is_ident("cfg") {
                Some("cfg")
            } else if attribute.path().is_ident("cfg_attr") {
                Some("cfg_attr")
            } else {
                None
            }
        });
        if let Some(attribute) = unsupported_attribute {
            return Err(BuildError::ResolverSource {
                path: source_path.to_owned(),
                source: syn::Error::new_spanned(
                    module,
                    format!("Resolver modules with #[{attribute}] are not supported"),
                ),
            });
        }
        let module_name = module.ident.unraw().to_string();
        if let Some((_, inline_items)) = &module.content {
            reconcile_external_children_in(
                source_path,
                inline_items,
                &module_directory.join(module_name),
                desired,
                fields,
                paths,
                writes,
            )?;
            continue;
        }
        let file_path = module_directory.join(format!("{module_name}.rs"));
        let mod_path = module_directory.join(&module_name).join("mod.rs");
        let path = match (source_exists(&file_path)?, source_exists(&mod_path)?) {
            (true, false) => file_path,
            (false, true) => mod_path,
            (true, true) => {
                return Err(BuildError::ResolverSource {
                    path: source_path.to_owned(),
                    source: syn::Error::new_spanned(
                        module,
                        format!(
                            "Resolver module is ambiguous; both {} and {} exist",
                            file_path.display(),
                            mod_path.display()
                        ),
                    ),
                });
            }
            (false, false) => {
                return Err(BuildError::ResolverSource {
                    path: source_path.to_owned(),
                    source: syn::Error::new_spanned(
                        module,
                        format!(
                            "Resolver module source does not exist; expected {} or {}",
                            file_path.display(),
                            mod_path.display()
                        ),
                    ),
                });
            }
        };
        let source = read_existing(&path)?.ok_or_else(|| BuildError::Io {
            path: path.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Resolver module source does not exist",
            ),
        })?;
        let ast = syn::parse_file(&source).map_err(|source| BuildError::ResolverSource {
            path: path.clone(),
            source,
        })?;
        let source_offset = if source.starts_with('\u{feff}') { 3 } else { 0 }
            + ast.shebang.as_ref().map_or(0, String::len);
        let mut edits = Vec::new();
        let mut anchor = None;
        for implementation in ast.items.iter().filter_map(|item| match item {
            Item::Impl(item) => Some(item),
            _ => None,
        }) {
            let Some(coordinate) = field_resolver_coordinate(implementation) else {
                continue;
            };
            let Some(resolver) = desired.get(&coordinate) else {
                anchor.get_or_insert(implementation);
                reconcile_stale_field_resolver(
                    implementation,
                    &coordinate,
                    &source,
                    source_offset,
                    &mut edits,
                )
                .map_err(|source| BuildError::ResolverSource {
                    path: path.clone(),
                    source,
                })?;
                continue;
            };
            if let Some(first_path) = fields.get(&coordinate) {
                return Err(BuildError::ResolverSource {
                    path: path.clone(),
                    source: syn::Error::new_spanned(
                        implementation,
                        format!(
                            "Duplicate field resolver implementations are ambiguous; found in {} and {}",
                            first_path.display(),
                            path.display()
                        ),
                    ),
                });
            }
            fields.insert(coordinate, path.clone());
            anchor.get_or_insert(implementation);
            reconcile_field_resolver(
                implementation,
                resolver,
                &RenderScope::from_items(&ast.items),
                &mut edits,
                false,
            )
            .map_err(|source| BuildError::ResolverSource {
                path: path.clone(),
                source,
            })?;
        }
        let inline_anchor = reconcile_inline_modules(
            &ast.items,
            desired,
            fields,
            &path,
            &source,
            source_offset,
            &mut edits,
        )
        .map_err(|source| BuildError::ResolverSource {
            path: path.clone(),
            source,
        })?;
        anchor = anchor.or(inline_anchor);
        let updated = match anchor {
            Some(anchor) => {
                apply_edits(&source, source_offset, anchor, edits).map_err(|source| {
                    BuildError::ResolverSource {
                        path: path.clone(),
                        source,
                    }
                })?
            }
            None => source.clone(),
        };
        syn::parse_file(&updated).map_err(|source| BuildError::ResolverSource {
            path: path.clone(),
            source,
        })?;
        reconcile_external_children(&path, &ast, desired, fields, paths, writes)?;
        paths.insert(path.clone());
        if updated != source {
            writes.push(PlannedWrite {
                path,
                original: Some(source),
                updated,
            });
        }
    }
    Ok(())
}

fn source_exists(path: &Path) -> Result<bool, BuildError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(BuildError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn is_cfg_test_module(module: &syn::ItemMod) -> bool {
    module.attrs.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<syn::Ident>()
                .is_ok_and(|condition| condition == "test")
    })
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
    path: &Path,
    source: &str,
    root_objects: &[syn::Ident],
    root_resolvers: &[syn::Ident],
    objects: &[syn::Ident],
    field_resolvers: &[FieldResolver],
    external_fields: &BTreeMap<(String, String), PathBuf>,
) -> syn::Result<String> {
    let ast = syn::parse_file(source)?;
    // parse_file removes these prefixes before assigning token byte ranges.
    let source_offset = if source.starts_with('\u{feff}') { 3 } else { 0 }
        + ast.shebang.as_ref().map_or(0, String::len);
    let implementation = query_resolver_implementation(&ast, &root_objects[0], &root_resolvers[0])?;
    let mut root_implementations = vec![Some(implementation)];
    for resolver in &root_resolvers[1..] {
        root_implementations.push(optional_root_resolver_implementation(
            &ast,
            resolver,
            "Multiple root resolver implementations are ambiguous",
        )?);
    }
    let query_generics = &implementation.generics;
    let query_context = resolver_context(implementation)?;
    let query_context_generics = context_generics(query_generics, query_context);
    let mut scope = RenderScope::from_items(&ast.items);
    scope.names.extend(
        objects
            .iter()
            .chain(root_objects)
            .map(|name| name.unraw().to_string()),
    );
    let mut edits = Vec::new();
    let desired_root_resolvers = root_resolvers
        .iter()
        .map(|resolver| resolver.unraw().to_string())
        .collect::<BTreeSet<_>>();
    for stale in ast.items.iter().filter_map(|item| match item {
        Item::Impl(implementation)
            if root_resolver_trait(implementation).is_some_and(|resolver| {
                !desired_root_resolvers.contains(&resolver.unraw().to_string())
            }) =>
        {
            Some(implementation)
        }
        _ => None,
    }) {
        if !generated_root_resolver_marker(stale, source, source_offset) {
            return Err(syn::Error::new_spanned(
                stale,
                "Cannot remove an application-written root resolver marker automatically",
            ));
        }
        edits.push(Edit {
            range: stale.span().byte_range(),
            replacement: String::new(),
        });
    }
    for (index, implementation) in root_implementations.iter().enumerate() {
        let Some(implementation) = implementation else {
            continue;
        };
        resolver_context(implementation)?;
        if !implementation.items.is_empty() {
            let message = if index == 0 {
                "Expected an empty query root resolver marker; grouped resolver methods are not supported"
            } else {
                "Expected an empty root resolver marker; grouped resolver methods are not supported"
            };
            return Err(syn::Error::new_spanned(&implementation.items[0], message));
        }
        scope
            .with_generics(&implementation.generics)
            .repair_bounds(&implementation.generics, &mut edits);
    }
    let mut field_resolvers = field_resolvers.to_vec();
    for ((object, _), implementation) in root_objects
        .iter()
        .zip(root_resolvers)
        .zip(&root_implementations)
    {
        let Some(implementation) = implementation else {
            continue;
        };
        for field_resolver in &mut field_resolvers {
            if field_resolver.object.unraw() == object.unraw() {
                field_resolver.receiver = implementation.self_ty.as_ref().clone();
            }
        }
    }
    let desired_fields = field_resolvers
        .iter()
        .map(|resolver| (resolver.coordinate(), resolver))
        .collect::<BTreeMap<_, _>>();
    let mut relocated_fields = external_fields.clone();
    let _ = reconcile_inline_modules(
        &ast.items,
        &desired_fields,
        &mut relocated_fields,
        path,
        source,
        source_offset,
        &mut edits,
    )?;
    let local_field_resolvers = field_resolvers
        .iter()
        .filter(|resolver| !relocated_fields.contains_key(&resolver.coordinate()))
        .cloned()
        .collect::<Vec<_>>();
    for implementation in ast.items.iter().filter_map(|item| match item {
        Item::Impl(item) => Some(item),
        _ => None,
    }) {
        if let Some(first_path) = field_resolver_coordinate(implementation)
            .and_then(|coordinate| relocated_fields.get(&coordinate))
        {
            return Err(syn::Error::new_spanned(
                implementation,
                format!(
                    "Duplicate field resolver implementations are ambiguous; found in {} and {}",
                    first_path.display(),
                    path.display()
                ),
            ));
        }
    }
    let desired_fields = reconcile_field_resolvers(
        &ast,
        &local_field_resolvers,
        &scope,
        source,
        source_offset,
        &mut edits,
    )?;

    let defined_types = ast
        .items
        .iter()
        .filter_map(item_type_name)
        .collect::<std::collections::BTreeSet<_>>();
    let root_declarations = root_objects
        .iter()
        .zip(&root_implementations)
        .filter(|(_, implementation)| implementation.is_none())
        .map(|(object, _)| object)
        .filter(|object| !defined_types.contains(&object.unraw().to_string()))
        .map(|object| format!("#[allow(non_camel_case_types)]\npub struct {object};"));
    let object_declarations = objects
        .iter()
        .filter(|object| !defined_types.contains(&object.unraw().to_string()))
        .map(|object| {
            format!("#[allow(non_camel_case_types)]\n#[derive(Clone, Copy)]\npub struct {object};")
        });
    let mut additions = root_declarations
        .chain(object_declarations)
        .collect::<Vec<_>>();
    for ((object, resolver), root_implementation) in root_objects
        .iter()
        .zip(root_resolvers)
        .zip(&root_implementations)
    {
        if root_implementation.is_some() {
            continue;
        }
        let (impl_generics, _, where_clause) = query_context_generics.split_for_impl();
        let root_implementation = quote! {
            #[allow(non_snake_case)]
            impl #impl_generics crate::generated::resolvers::#resolver<#query_context>
                for self::#object #where_clause
            {}
        };
        additions.push(root_implementation.to_string());
    }
    for resolver in desired_fields.into_values() {
        let root_index = root_objects
            .iter()
            .position(|object| object.unraw() == resolver.object.unraw());
        let (generics, context) = match root_index {
            Some(index) => match root_implementations[index] {
                Some(root_implementation) => (
                    &root_implementation.generics,
                    resolver_context(root_implementation)?,
                ),
                None => (&query_context_generics, query_context),
            },
            None => (&implementation.generics, resolver_context(implementation)?),
        };
        additions.push(render_field_resolver(resolver, generics, context, &scope));
    }
    let additions = additions.join("\n\n");
    if !additions.is_empty() {
        edits.push(Edit {
            range: source.len() - source_offset..source.len() - source_offset,
            replacement: format!("\n{additions}\n"),
        });
    }

    apply_edits(source, source_offset, implementation, edits)
}

fn reconcile_inline_modules<'a>(
    items: &'a [Item],
    desired_fields: &BTreeMap<(String, String), &FieldResolver>,
    relocated_fields: &mut BTreeMap<(String, String), PathBuf>,
    source_path: &Path,
    source: &str,
    source_offset: usize,
    edits: &mut Vec<Edit>,
) -> syn::Result<Option<&'a syn::ItemImpl>> {
    let mut anchor = None;
    for inline_items in items.iter().filter_map(|item| match item {
        Item::Mod(module) if !is_cfg_test_module(module) => {
            module.content.as_ref().map(|(_, items)| items)
        }
        _ => None,
    }) {
        for implementation in inline_items.iter().filter_map(|item| match item {
            Item::Impl(item) => Some(item),
            _ => None,
        }) {
            let Some(coordinate) = field_resolver_coordinate(implementation) else {
                continue;
            };
            let Some(resolver) = desired_fields.get(&coordinate) else {
                anchor.get_or_insert(implementation);
                reconcile_stale_field_resolver(
                    implementation,
                    &coordinate,
                    source,
                    source_offset,
                    edits,
                )?;
                continue;
            };
            if let Some(first_path) = relocated_fields.get(&coordinate) {
                return Err(syn::Error::new_spanned(
                    implementation,
                    format!(
                        "Duplicate field resolver implementations are ambiguous; found in {} and {}",
                        first_path.display(),
                        source_path.display()
                    ),
                ));
            }
            relocated_fields.insert(coordinate, source_path.to_owned());
            anchor.get_or_insert(implementation);
            reconcile_field_resolver(
                implementation,
                resolver,
                &RenderScope::from_items(inline_items),
                edits,
                false,
            )?;
        }
        if let Some(nested_anchor) = reconcile_inline_modules(
            inline_items,
            desired_fields,
            relocated_fields,
            source_path,
            source,
            source_offset,
            edits,
        )? {
            anchor.get_or_insert(nested_anchor);
        }
    }
    Ok(anchor)
}

fn query_resolver_implementation<'a>(
    ast: &'a syn::File,
    object: &syn::Ident,
    resolver: &syn::Ident,
) -> syn::Result<&'a syn::ItemImpl> {
    optional_root_resolver_implementation(
        ast,
        resolver,
        "Multiple query resolver implementations are ambiguous",
    )?
    .ok_or_else(|| syn::Error::new(
        object.span(),
        "Expected one explicit crate::generated::resolvers implementation for the query root; aliases are not resolved",
    ))
}

fn optional_root_resolver_implementation<'a>(
    ast: &'a syn::File,
    resolver: &syn::Ident,
    duplicate_message: &str,
) -> syn::Result<Option<&'a syn::ItemImpl>> {
    let mut implementations = ast
        .items
        .iter()
        .filter_map(|item| matching_query_resolver(item, resolver));
    let Some(implementation) = implementations.next() else {
        return Ok(None);
    };
    if implementations.next().is_some() {
        return Err(syn::Error::new_spanned(implementation, duplicate_message));
    }
    Ok(Some(implementation))
}

fn root_resolver_trait(implementation: &syn::ItemImpl) -> Option<&syn::Ident> {
    let (_, path, _) = implementation.trait_.as_ref()?;
    let segments = path.segments.iter().collect::<Vec<_>>();
    let [crate_name, generated, resolvers, resolver] = segments.as_slice() else {
        return None;
    };
    ([
        crate_name.ident.unraw().to_string(),
        generated.ident.unraw().to_string(),
        resolvers.ident.unraw().to_string(),
    ] == ["crate", "generated", "resolvers"]
        && resolver.ident.unraw().to_string().ends_with("Resolver"))
    .then_some(&resolver.ident)
}

fn generated_root_resolver_marker(
    implementation: &syn::ItemImpl,
    source: &str,
    source_offset: usize,
) -> bool {
    let Some(resolver) = root_resolver_trait(implementation) else {
        return false;
    };
    let name = resolver.unraw().to_string();
    let Some(object) = name.strip_suffix("Resolver") else {
        return false;
    };
    let object = syn::parse_str::<syn::Ident>(object)
        .ok()
        .filter(|_| object != "gen")
        .unwrap_or_else(|| format_ident!("r#{}", object));
    let unqualified: syn::ItemImpl = syn::parse_quote! {
        #[allow(non_snake_case)]
        impl<C: Sync> crate::generated::resolvers::#resolver<C> for self::#object {}
    };
    let qualified: syn::ItemImpl = syn::parse_quote! {
        #[allow(non_snake_case)]
        impl<C: ::core::marker::Sync> crate::generated::resolvers::#resolver<C>
            for self::#object
        {}
    };
    let tokens = quote!(#implementation).to_string();
    let generated = [unqualified, qualified]
        .iter()
        .any(|expected| quote!(#expected).to_string() == tokens);
    let range = implementation.span().byte_range();
    let source_range = range.start + source_offset..range.end + source_offset;
    generated
        && source.get(source_range).is_some_and(|implementation| {
            !implementation.contains("//") && !implementation.contains("/*")
        })
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

fn context_generics(generics: &syn::Generics, context: &Type) -> syn::Generics {
    let names = generics
        .params
        .iter()
        .map(|parameter| match parameter {
            syn::GenericParam::Lifetime(parameter) => parameter.lifetime.ident.to_string(),
            syn::GenericParam::Type(parameter) => parameter.ident.to_string(),
            syn::GenericParam::Const(parameter) => parameter.ident.to_string(),
        })
        .collect::<BTreeSet<_>>();
    let retained = generic_references(&names, |visitor| visitor.visit_type(context));
    let mut filtered = generics.clone();
    filtered.params = filtered
        .params
        .into_iter()
        .filter(|parameter| {
            let name = match parameter {
                syn::GenericParam::Lifetime(parameter) => parameter.lifetime.ident.to_string(),
                syn::GenericParam::Type(parameter) => parameter.ident.to_string(),
                syn::GenericParam::Const(parameter) => parameter.ident.to_string(),
            };
            retained.contains(&name)
        })
        .collect();
    for parameter in &mut filtered.params {
        match parameter {
            syn::GenericParam::Lifetime(parameter) => {
                parameter.bounds = parameter
                    .bounds
                    .clone()
                    .into_iter()
                    .filter(|bound| {
                        generic_references(&names, |visitor| {
                            visitor.visit_lifetime(bound);
                        })
                        .is_subset(&retained)
                    })
                    .collect()
            }
            syn::GenericParam::Type(parameter) => {
                parameter.bounds = parameter
                    .bounds
                    .clone()
                    .into_iter()
                    .filter(|bound| {
                        generic_references(&names, |visitor| {
                            visitor.visit_type_param_bound(bound);
                        })
                        .is_subset(&retained)
                    })
                    .collect()
            }
            syn::GenericParam::Const(_) => {}
        }
    }
    if let Some(where_clause) = &mut filtered.where_clause {
        where_clause.predicates = where_clause
            .predicates
            .clone()
            .into_iter()
            .filter(|predicate| {
                generic_references(&names, |visitor| {
                    visitor.visit_where_predicate(predicate);
                })
                .is_subset(&retained)
            })
            .collect();
        if where_clause.predicates.is_empty() {
            filtered.where_clause = None;
        }
    }
    filtered
}

fn generic_references(
    names: &BTreeSet<String>,
    visit: impl FnOnce(&mut GenericReferences<'_>),
) -> BTreeSet<String> {
    let mut references = GenericReferences {
        names,
        references: BTreeSet::new(),
    };
    visit(&mut references);
    references.references
}

struct GenericReferences<'a> {
    names: &'a BTreeSet<String>,
    references: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for GenericReferences<'_> {
    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if path.qself.is_none()
            && let Some(segment) = path.path.segments.first()
            && self.names.contains(&segment.ident.to_string())
        {
            self.references.insert(segment.ident.to_string());
        }
        syn::visit::visit_expr_path(self, path);
    }

    fn visit_lifetime(&mut self, lifetime: &'ast syn::Lifetime) {
        let name = lifetime.ident.to_string();
        if self.names.contains(&name) {
            self.references.insert(name);
        }
    }

    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        if path.qself.is_none()
            && let Some(segment) = path.path.segments.first()
            && self.names.contains(&segment.ident.to_string())
        {
            self.references.insert(segment.ident.to_string());
        }
        syn::visit::visit_type_path(self, path);
    }
}

fn reconcile_field_resolvers<'a>(
    ast: &syn::File,
    field_resolvers: &'a [FieldResolver],
    scope: &RenderScope,
    source: &str,
    source_offset: usize,
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
            reconcile_field_resolver(item, resolver, scope, edits, true)?;
        } else {
            reconcile_stale_field_resolver(item, &coordinate, source, source_offset, edits)?;
        }
    }
    Ok(desired_fields)
}

fn reconcile_stale_field_resolver(
    implementation: &syn::ItemImpl,
    coordinate: &(String, String),
    source: &str,
    source_offset: usize,
    edits: &mut Vec<Edit>,
) -> syn::Result<()> {
    let (_, resolve) = field_resolver_items(implementation)?;
    validate_resolve(resolve)?;
    let semantic_stub = matches!(
        resolve.block.stmts.as_slice(),
        [syn::Stmt::Expr(syn::Expr::Macro(expression), None)]
            if expression.attrs.is_empty()
                && expression.mac.path.leading_colon.is_some()
                && expression.mac.path.segments.iter().map(|segment| segment.ident.unraw().to_string()).eq(["core", "unimplemented"].map(str::to_owned))
                && expression.mac.tokens.is_empty()
    );
    let range = implementation.span().byte_range();
    let source_range = range.start + source_offset..range.end + source_offset;
    let has_comments = source.get(source_range).is_none_or(|implementation| {
        implementation.contains("//") || implementation.contains("/*")
    });
    let generated_stub = semantic_stub
        && implementation.attrs.is_empty()
        && resolve.attrs.is_empty()
        && !has_comments;
    if !generated_stub {
        return Err(syn::Error::new_spanned(
            resolve,
            format!(
                "Cannot remove application-written resolver {}.{} automatically",
                coordinate.0, coordinate.1
            ),
        ));
    }
    edits.push(Edit {
        range: implementation.span().byte_range(),
        replacement: String::new(),
    });
    Ok(())
}

fn reconcile_field_resolver(
    implementation: &syn::ItemImpl,
    resolver: &FieldResolver,
    scope: &RenderScope,
    edits: &mut Vec<Edit>,
    update_receiver: bool,
) -> syn::Result<()> {
    let scope = scope.with_generics(&implementation.generics);
    scope.repair_bounds(&implementation.generics, edits);
    if let Some((_, path, _)) = &implementation.trait_ {
        scope.repair_path(path, edits);
    }
    if update_receiver {
        let current_receiver = &implementation.self_ty;
        let desired_receiver =
            receiver_with_existing_arguments(&resolver.receiver, current_receiver)?;
        if !equivalent_output_type(current_receiver, &desired_receiver) {
            edits.push(Edit {
                range: implementation.self_ty.span().byte_range(),
                replacement: quote!(#desired_receiver).to_string(),
            });
        }
    }
    let (output, resolve) = field_resolver_items(implementation)?;
    validate_resolve(resolve)?;
    reject_removed_argument_uses(resolve, resolver)?;
    if !equivalent_output_type(&output.ty, &resolver.output) {
        let replacement = &resolver.output;
        edits.push(Edit {
            range: output.ty.span().byte_range(),
            replacement: scope.render_type(replacement),
        });
    } else {
        scope.repair_type(&output.ty, edits);
    }
    if let syn::ReturnType::Type(_, output) = &resolve.sig.output {
        scope.repair_type(output, edits);
    }
    Ok(())
}

fn reject_removed_argument_uses(
    resolve: &syn::ImplItemFn,
    resolver: &FieldResolver,
) -> syn::Result<()> {
    let binding = resolve
        .sig
        .inputs
        .iter()
        .nth(2)
        .and_then(|argument| match argument {
            syn::FnArg::Typed(argument) => match argument.pat.as_ref() {
                syn::Pat::Ident(binding) => Some(&binding.ident),
                _ => None,
            },
            _ => None,
        });
    let Some(binding) = binding else {
        return Ok(());
    };
    let mut visitor = RemovedArgumentUse {
        binding,
        arguments: &resolver.arguments,
        removed: None,
    };
    visitor.visit_block(&resolve.block);
    let Some((argument, span)) = visitor.removed else {
        return Ok(());
    };
    let (object, field) = resolver.coordinate();
    Err(syn::Error::new(
        span,
        format!("Cannot preserve resolver {object}.{field}; argument {argument} is not in the SDL"),
    ))
}

struct RemovedArgumentUse<'a> {
    binding: &'a syn::Ident,
    arguments: &'a BTreeSet<String>,
    removed: Option<(String, proc_macro2::Span)>,
}

impl<'ast> Visit<'ast> for RemovedArgumentUse<'_> {
    fn visit_expr_field(&mut self, expression: &'ast syn::ExprField) {
        if self.removed.is_none()
            && matches!(expression.base.as_ref(), syn::Expr::Path(path) if path.qself.is_none() && path.path.is_ident(self.binding))
            && let syn::Member::Named(member) = &expression.member
            && !self.arguments.contains(&member.unraw().to_string())
        {
            self.removed = Some((member.unraw().to_string(), member.span()));
            return;
        }
        syn::visit::visit_expr_field(self, expression);
    }
}

fn receiver_with_existing_arguments(desired: &Type, current: &Type) -> syn::Result<Type> {
    let (Type::Path(desired_path), Type::Path(current_path)) = (desired, current) else {
        return Ok(desired.clone());
    };
    let mut desired_path = desired_path.clone();
    let desired_arguments = &desired_path
        .path
        .segments
        .last()
        .expect("Rust type paths have at least one segment")
        .arguments;
    let current_arguments = &current_path
        .path
        .segments
        .last()
        .expect("Rust type paths have at least one segment")
        .arguments;
    if !compatible_generic_arguments(desired_arguments, current_arguments) {
        return Err(syn::Error::new_spanned(
            current,
            "Cannot preserve field resolver generic bindings for the query root type",
        ));
    }
    desired_path.path.segments.last_mut().unwrap().arguments = current_arguments.clone();
    Ok(Type::Path(desired_path))
}

fn compatible_generic_arguments(left: &PathArguments, right: &PathArguments) -> bool {
    match (left, right) {
        (PathArguments::None, PathArguments::None) => true,
        (PathArguments::AngleBracketed(left), PathArguments::AngleBracketed(right)) => {
            left.args.len() == right.args.len()
                && left.args.iter().zip(&right.args).all(|(left, right)| {
                    matches!(
                        (left, right),
                        (GenericArgument::Lifetime(_), GenericArgument::Lifetime(_))
                            | (GenericArgument::Type(_), GenericArgument::Type(_))
                            | (GenericArgument::Const(_), GenericArgument::Const(_))
                    )
                })
        }
        _ => false,
    }
}

fn field_resolver_items(
    implementation: &syn::ItemImpl,
) -> syn::Result<(&syn::ImplItemType, &syn::ImplItemFn)> {
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
    let mut resolves = implementation.items.iter().filter_map(|item| match item {
        syn::ImplItem::Fn(item) if item.sig.ident == "resolve" => Some(item),
        _ => None,
    });
    let resolve = resolves.next().ok_or_else(|| {
        syn::Error::new_spanned(
            implementation,
            "Expected one resolve method in a field resolver implementation",
        )
    })?;
    if resolves.next().is_some() {
        return Err(syn::Error::new_spanned(
            implementation,
            "Expected one resolve method in a field resolver implementation",
        ));
    }
    if implementation.items.iter().any(|item| {
        !matches!(item, syn::ImplItem::Type(item) if item.ident == "Output")
            && !matches!(item, syn::ImplItem::Fn(item) if item.sig.ident == "resolve")
    }) {
        return Err(syn::Error::new_spanned(
            implementation,
            "Unexpected item in a field resolver implementation",
        ));
    }
    Ok((output, resolve))
}

fn validate_resolve(resolve: &syn::ImplItemFn) -> syn::Result<()> {
    let mut inputs = resolve.sig.inputs.iter();
    let receiver = matches!(
        inputs.next(),
        Some(syn::FnArg::Receiver(receiver))
            if receiver.reference.is_some() && receiver.mutability.is_none()
    );
    let context = matches!(
        inputs.next(),
        Some(syn::FnArg::Typed(context))
            if matches!(context.ty.as_ref(), Type::Reference(reference) if reference.mutability.is_none())
    );
    let args = matches!(inputs.next(), Some(syn::FnArg::Typed(_)));
    if !receiver || !context || !args || inputs.next().is_some() {
        return Err(syn::Error::new_spanned(
            &resolve.sig,
            "Expected resolve with self, Context, and Args parameters",
        ));
    }
    if matches!(resolve.sig.output, syn::ReturnType::Default) {
        return Err(syn::Error::new_spanned(
            &resolve.sig,
            "Expected resolve to declare its result type",
        ));
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
            let exact_path = left.path.segments.len() == right.path.segments.len()
                && left
                    .path
                    .segments
                    .iter()
                    .zip(&right.path.segments)
                    .all(|(left, right)| {
                        left.ident.unraw() == right.ident.unraw()
                            && equivalent_path_arguments(&left.arguments, &right.arguments)
                    });
            exact_path
                || builtin_path(&left.path) == builtin_path(&right.path)
                    && builtin_path(&left.path).is_some()
                    && equivalent_path_arguments(
                        &left.path.segments.last().unwrap().arguments,
                        &right.path.segments.last().unwrap().arguments,
                    )
        }
        _ => quote!(#left).to_string() == quote!(#right).to_string(),
    }
}

fn builtin_path(path: &syn::Path) -> Option<&'static str> {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.unraw().to_string())
        .collect::<Vec<_>>();
    match segments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["String"] | ["std", "string", "String"] => Some("String"),
        ["bool"] | ["core", "primitive", "bool"] => Some("bool"),
        ["i32"] | ["core", "primitive", "i32"] => Some("i32"),
        ["f64"] | ["core", "primitive", "f64"] => Some("f64"),
        ["Option"] | ["core", "option", "Option"] => Some("Option"),
        ["Vec"] | ["std", "vec", "Vec"] => Some("Vec"),
        ["Result"] | ["core", "result", "Result"] => Some("Result"),
        ["Sync"] | ["core", "marker", "Sync"] => Some("Sync"),
        _ => None,
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
    arguments: BTreeSet<String>,
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
    arguments: BTreeSet<String>,
    ty: &GraphqlType,
    root_names: &[&str],
) -> Result<FieldResolver, BuildError> {
    let output = resolver_output_type(schema, object_name, field_name, ty, root_names)
        .map_err(BuildError::Codegen)?;
    let object = rust_ident(object_name);
    Ok(FieldResolver {
        receiver: syn::parse_quote!(self::#object),
        object,
        field: rust_ident(field_name),
        arguments,
        output: syn::parse2(output).expect("generated resolver output types must parse"),
    })
}

fn resolver_output_type(
    schema: &Schema,
    object_name: &str,
    field_name: &str,
    ty: &GraphqlType,
    root_names: &[&str],
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
    match composite_resolver_output_type(schema, ty, root_names) {
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
    root_names: &[&str],
) -> Option<proc_macro2::TokenStream> {
    match ty {
        GraphqlType::NonNullNamed(name) => {
            named_composite_resolver_output_type(schema, name, root_names)
        }
        GraphqlType::Named(name) => {
            let output = named_composite_resolver_output_type(schema, name, root_names)?;
            Some(quote! { ::core::option::Option<#output> })
        }
        GraphqlType::NonNullList(item) => {
            let item = composite_resolver_output_type(schema, item, root_names)?;
            Some(quote! { ::std::vec::Vec<#item> })
        }
        GraphqlType::List(item) => {
            let item = composite_resolver_output_type(schema, item, root_names)?;
            Some(quote! { ::core::option::Option<::std::vec::Vec<#item>> })
        }
    }
}

fn named_composite_resolver_output_type(
    schema: &Schema,
    name: &apollo_compiler::Name,
    root_names: &[&str],
) -> Option<proc_macro2::TokenStream> {
    match schema.types.get(name)? {
        ExtendedType::Object(_) if !root_names.contains(&name.as_str()) => {
            let object = rust_ident(name.as_str());
            Some(quote! { self::#object })
        }
        ExtendedType::Interface(_) | ExtendedType::Union(_) => {
            let abstract_type = rust_ident(name.as_str());
            let members = codegen::abstract_member_names(schema, name)?
                .into_iter()
                .map(|member| {
                    (!root_names.contains(&member)).then(|| {
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
    let mapped = codegen::rust_name(name);
    // syn accepts `gen` independently of the consumer's Rust edition.
    syn::parse_str::<syn::Ident>(&mapped)
        .ok()
        .filter(|_| mapped != "gen")
        .unwrap_or_else(|| format_ident!("r#{}", mapped))
}

#[derive(Clone, Default)]
struct RenderScope {
    names: BTreeSet<String>,
    existing_bindings: BTreeSet<String>,
    glob: bool,
}

impl RenderScope {
    fn from_items(items: &[Item]) -> Self {
        let mut scope = Self::default();
        for item in items {
            if let Some(name) = item_type_name(item) {
                scope.names.insert(name);
            }
            match item {
                Item::Use(item) => scope.imports(&item.tree, None),
                Item::Mod(item) => {
                    scope.names.insert(item.ident.unraw().to_string());
                }
                Item::Trait(item) => {
                    scope.names.insert(item.ident.unraw().to_string());
                }
                Item::TraitAlias(item) => {
                    scope.names.insert(item.ident.unraw().to_string());
                }
                Item::ExternCrate(item) => {
                    scope.names.insert(
                        item.rename
                            .as_ref()
                            .map_or(&item.ident, |(_, name)| name)
                            .unraw()
                            .to_string(),
                    );
                }
                _ => {}
            }
        }
        scope.existing_bindings = scope.names.clone();
        scope
    }

    fn imports(&mut self, tree: &syn::UseTree, parent: Option<&syn::Ident>) {
        match tree {
            syn::UseTree::Name(name) => {
                let ident = if name.ident == "self" {
                    parent.unwrap_or(&name.ident)
                } else {
                    &name.ident
                };
                self.names.insert(ident.unraw().to_string());
            }
            syn::UseTree::Rename(name) => {
                self.names.insert(name.rename.unraw().to_string());
            }
            syn::UseTree::Path(path) => self.imports(&path.tree, Some(&path.ident)),
            syn::UseTree::Group(group) => group
                .items
                .iter()
                .for_each(|tree| self.imports(tree, parent)),
            syn::UseTree::Glob(_) => self.glob = true,
        }
    }

    fn with_generics(&self, generics: &syn::Generics) -> Self {
        let mut scope = self.clone();
        scope.names.extend(
            generics
                .type_params()
                .map(|param| param.ident.unraw().to_string()),
        );
        scope.existing_bindings.extend(
            generics
                .type_params()
                .map(|param| param.ident.unraw().to_string()),
        );
        scope
    }

    fn blocked(&self, name: &str) -> bool {
        self.glob || self.names.contains(name)
    }

    fn standard(&self, name: &str) -> String {
        if self.blocked(name) {
            standard_path(name).to_owned()
        } else {
            name.to_owned()
        }
    }

    fn repair_path(&self, path: &syn::Path, edits: &mut Vec<Edit>) {
        if path.leading_colon.is_some() {
            return;
        }
        let Some(first) = path.segments.first() else {
            return;
        };
        let name = first.ident.unraw().to_string();
        // Existing bindings may be application aliases; do not reinterpret them.
        if !self.blocked(&name) || self.existing_bindings.contains(&name) || self.glob {
            return;
        }
        if path.segments.len() == 1 && builtin_path(path).is_some() {
            edits.push(Edit {
                range: first.ident.span().byte_range(),
                replacement: standard_path(&name).to_owned(),
            });
        } else if name == "necrassrs" || builtin_path(path).is_some() {
            let start = first.ident.span().byte_range().start;
            edits.push(Edit {
                range: start..start,
                replacement: "::".to_owned(),
            });
        }
    }

    fn repair_type(&self, ty: &Type, edits: &mut Vec<Edit>) {
        match ty {
            Type::Path(ty) if ty.qself.is_none() => {
                self.repair_path(&ty.path, edits);
                for segment in &ty.path.segments {
                    if let PathArguments::AngleBracketed(arguments) = &segment.arguments {
                        for argument in &arguments.args {
                            if let GenericArgument::Type(ty) = argument {
                                self.repair_type(ty, edits);
                            }
                        }
                    }
                }
            }
            Type::Paren(ty) => self.repair_type(&ty.elem, edits),
            Type::Group(ty) => self.repair_type(&ty.elem, edits),
            _ => {}
        }
    }

    fn repair_bounds(&self, generics: &syn::Generics, edits: &mut Vec<Edit>) {
        for bound in generics
            .type_params()
            .flat_map(|param| &param.bounds)
            .chain(
                generics
                    .where_clause
                    .iter()
                    .flat_map(|clause| &clause.predicates)
                    .filter_map(|predicate| match predicate {
                        syn::WherePredicate::Type(predicate) => Some(&predicate.bounds),
                        _ => None,
                    })
                    .flatten(),
            )
        {
            if let syn::TypeParamBound::Trait(bound) = bound
                && builtin_path(&bound.path) == Some("Sync")
            {
                self.repair_path(&bound.path, edits);
            }
        }
    }

    fn render_type(&self, ty: &Type) -> String {
        let Type::Path(ty) = ty else {
            return quote!(#ty).to_string();
        };
        if ty.qself.is_some() {
            return quote!(#ty).to_string();
        }
        let path = &ty.path;
        let segments = path
            .segments
            .iter()
            .map(|segment| {
                let ident = &segment.ident;
                let plain = ident.unraw().to_string();
                let name = if plain != "gen" && syn::parse_str::<syn::Ident>(&plain).is_ok() {
                    plain
                } else {
                    ident.to_string()
                };
                match &segment.arguments {
                    PathArguments::None => name,
                    PathArguments::AngleBracketed(arguments) => {
                        let arguments = arguments
                            .args
                            .iter()
                            .map(|argument| match argument {
                                GenericArgument::Type(ty) => self.render_type(ty),
                                _ => quote!(#argument).to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("{name}<{arguments}>")
                    }
                    arguments => format!("{name}{}", quote!(#arguments)),
                }
            })
            .collect::<Vec<_>>();
        if path.leading_colon.is_some()
            && let Some(name) = builtin_path(path)
        {
            let last = segments.last().unwrap();
            return format!(
                "{}{}",
                self.standard(name),
                last.strip_prefix(name).unwrap()
            );
        }
        let prefix = if path.leading_colon.is_some()
            && (segments.first().is_none_or(|name| name != "necrassrs")
                || self.blocked("necrassrs"))
        {
            "::"
        } else {
            ""
        };
        format!("{prefix}{}", segments.join("::"))
    }
}

impl VisitMut for RenderScope {
    fn visit_trait_bound_mut(&mut self, bound: &mut syn::TraitBound) {
        if builtin_path(&bound.path) == Some("Sync")
            && self.blocked("Sync")
            && !self.existing_bindings.contains("Sync")
            && !self.glob
        {
            bound.path = syn::parse_quote!(::core::marker::Sync);
        }
        syn::visit_mut::visit_trait_bound_mut(self, bound);
    }
}

fn standard_path(name: &str) -> &'static str {
    match name {
        "String" => "::std::string::String",
        "Vec" => "::std::vec::Vec",
        "Option" => "::core::option::Option",
        "Result" => "::core::result::Result",
        "Sync" => "::core::marker::Sync",
        "bool" => "::core::primitive::bool",
        "i32" => "::core::primitive::i32",
        "f64" => "::core::primitive::f64",
        _ => unreachable!("only recognized standard types have canonical paths"),
    }
}

fn render_field_resolver(
    resolver: &FieldResolver,
    generics: &syn::Generics,
    context: &Type,
    scope: &RenderScope,
) -> String {
    let scope = scope.with_generics(generics);
    let framework = if scope.blocked("necrassrs") {
        "::necrassrs"
    } else {
        "necrassrs"
    };
    let result = scope.standard("Result");
    let object = &resolver.object;
    let field = &resolver.field;
    let receiver = &resolver.receiver;
    let output = &resolver.output;
    let mut generics = generics.clone();
    scope.clone().visit_generics_mut(&mut generics);
    let parameters = generics
        .params
        .iter()
        .map(|parameter| match parameter {
            syn::GenericParam::Type(param) if param.attrs.is_empty() && param.default.is_none() => {
                let name = &param.ident;
                if param.bounds.is_empty() {
                    return name.to_string();
                }
                let bounds = param
                    .bounds
                    .iter()
                    .map(|bound| match bound {
                        syn::TypeParamBound::Trait(bound) if bound.lifetimes.is_none() => {
                            let modifier = &bound.modifier;
                            let path = &bound.path;
                            format!(
                                "{}{}",
                                quote!(#modifier),
                                scope.render_type(&syn::parse_quote!(#path))
                            )
                        }
                        _ => quote!(#bound).to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" + ");
                format!("{name}: {bounds}")
            }
            _ => quote!(#parameter).to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let impl_generics = if parameters.is_empty() {
        String::new()
    } else {
        format!("<{parameters}>")
    };
    let where_clause = &generics.where_clause;
    format!(
        "impl{} {framework}::Resolver<crate::generated::fields::{object}::{field}, {}> for {} {}{{\n\
         \x20   type Output = {};\n\n\
         \x20   async fn resolve(\n\
         \x20       &self,\n\
         \x20       _context: &{},\n\
         \x20       _args: crate::generated::types::{object}::{field}::Args,\n\
         \x20   ) -> {result}<Self::Output, {framework}::ResolverError> {{\n\
         \x20       ::core::unimplemented!()\n\
         \x20   }}\n\
         }}",
        impl_generics,
        scope.render_type(context),
        scope.render_type(receiver),
        quote!(#where_clause),
        scope.render_type(output),
        scope.render_type(context),
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
    use quote::ToTokens;
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

    fn field_implementation_mut<'a>(
        ast: &'a mut syn::File,
        object: &str,
        field: &str,
    ) -> &'a mut syn::ItemImpl {
        ast.items
            .iter_mut()
            .find_map(|item| match item {
                Item::Impl(item)
                    if field_resolver_coordinate(item)
                        == Some((object.to_owned(), field.to_owned())) =>
                {
                    Some(item)
                }
                _ => None,
            })
            .unwrap()
    }

    fn field_output(source: &str, object: &str, field: &str) -> String {
        let ast = syn::parse_file(source).unwrap();
        field_output_in(&ast.items, object, field)
    }

    fn field_implementation_in<'a>(
        items: &'a [Item],
        object: &str,
        field: &str,
    ) -> Option<&'a syn::ItemImpl> {
        items.iter().find_map(|item| match item {
            Item::Impl(item)
                if field_resolver_coordinate(item)
                    == Some((object.to_owned(), field.to_owned())) =>
            {
                Some(item)
            }
            Item::Mod(module) => module
                .content
                .as_ref()
                .and_then(|(_, items)| field_implementation_in(items, object, field)),
            _ => None,
        })
    }

    fn field_output_in(items: &[Item], object: &str, field: &str) -> String {
        let implementation = field_implementation_in(items, object, field).unwrap();
        let output = implementation
            .items
            .iter()
            .find_map(|item| match item {
                syn::ImplItem::Type(item) if item.ident == "Output" => Some(&item.ty),
                _ => None,
            })
            .unwrap();
        quote!(#output).to_string()
    }

    fn mutate_resolve(source: &str, change: fn(&mut syn::ImplItemFn)) -> String {
        let mut ast = syn::parse_file(source).unwrap();
        let implementation = field_implementation_mut(&mut ast, "Query", "hello");
        let resolve = implementation
            .items
            .iter_mut()
            .find_map(|item| match item {
                syn::ImplItem::Fn(method) if method.sig.ident == "resolve" => Some(method),
                _ => None,
            })
            .unwrap();
        change(resolve);
        ast.to_token_stream().to_string()
    }

    fn duplicate_resolve(source: &str) -> String {
        let mut ast = syn::parse_file(source).unwrap();
        let implementation = field_implementation_mut(&mut ast, "Query", "hello");
        let resolve = implementation
            .items
            .iter()
            .find(|item| matches!(item, syn::ImplItem::Fn(method) if method.sig.ident == "resolve"))
            .unwrap()
            .clone();
        implementation.items.push(resolve);
        ast.to_token_stream().to_string()
    }

    fn write_external_resolver(file: &ResolverFile, relative_path: &str) -> std::path::PathBuf {
        let child = file.directory.join(relative_path);
        fs::create_dir_all(child.parent().unwrap()).unwrap();
        fs::write(
            &file.path,
            r#"
pub struct Query;

mod fields;

impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for self::Query {}
"#,
        )
        .unwrap();
        fs::write(
            &child,
            r#"
impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::Query::hello, C>
    for super::Query
{
    type Output = ::std::string::String;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::Query::hello::Args,
    ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
        /* retain this body */
        ::core::unimplemented!()
    }
}
"#,
        )
        .unwrap();
        child
    }

    fn write_nested_external_resolver(
        file: &ResolverFile,
        module_path: &str,
    ) -> std::path::PathBuf {
        let child = write_external_resolver(file, "resolvers/group/fields.rs");
        fs::write(&file.path, file.read().replace("mod fields;", "mod group;")).unwrap();
        fs::write(file.directory.join(module_path), "mod fields;\n").unwrap();
        fs::write(
            &child,
            fs::read_to_string(&child)
                .unwrap()
                .replace("for super::Query", "for super::super::Query"),
        )
        .unwrap();
        child
    }

    fn write_external_resolver_below_inline_module(file: &ResolverFile) -> std::path::PathBuf {
        let child = write_external_resolver(file, "resolvers/group/fields.rs");
        fs::write(
            &file.path,
            file.read()
                .replace("mod fields;", "mod group {\n    mod fields;\n}"),
        )
        .unwrap();
        fs::write(
            &child,
            fs::read_to_string(&child)
                .unwrap()
                .replace("for super::Query", "for super::super::Query"),
        )
        .unwrap();
        child
    }

    fn write_inline_resolver_below_external_module(file: &ResolverFile) -> std::path::PathBuf {
        let child = write_external_resolver(file, "resolvers/group.rs");
        fs::write(&file.path, file.read().replace("mod fields;", "mod group;")).unwrap();
        let source = fs::read_to_string(&child)
            .unwrap()
            .replace("for super::Query", "for super::super::Query");
        fs::write(&child, format!("mod fields {{\n{source}\n}}\n")).unwrap();
        child
    }

    fn assert_unsupported_module_syntax_is_rejected(declaration: &str, expected: &str) {
        let file = ResolverFile::new();
        let child = write_external_resolver(&file, "resolvers/fields.rs");
        let entry_source = file.read().replace("mod fields;", declaration);
        fs::write(&file.path, &entry_source).unwrap();
        let child_source = fs::read_to_string(&child).unwrap();

        let error = file
            .synchronize("type Query { hello: Int! }")
            .expect_err("unsupported module syntax unexpectedly succeeded");

        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(file.read(), entry_source);
        assert_eq!(fs::read_to_string(child).unwrap(), child_source);
    }

    fn assert_external_resolver_is_updated(file: &ResolverFile, child: &Path, receiver: &str) {
        file.synchronize("type Query { hello: Int! }").unwrap();

        let child_source = fs::read_to_string(child).unwrap();
        assert_eq!(field_output(&child_source, "Query", "hello"), "i32");
        assert!(child_source.contains("/* retain this body */"));
        assert!(child_source.contains(receiver));
        assert!(!file.read().contains("fields::Query::hello"));

        let entry_source = file.read();
        file.synchronize("type Query { hello: Int! }").unwrap();
        assert_eq!(fs::read_to_string(child).unwrap(), child_source);
        assert_eq!(file.read(), entry_source);
    }

    #[test]
    fn readable_rendering_preserves_existing_raw_receivers_and_user_source() {
        let file = ResolverFile::new();
        let sdl = "type Query { operator: Operator! } type Operator { name: String! }";
        file.synchronize(sdl).unwrap();
        let source = file
            .read()
            .replace(
                "pub struct Operator;",
                "pub struct Operator { pub state: u8 }",
            )
            .replace(
                "for self::Operator",
                "for self:: /* representation */ r#Operator",
            )
            .replace(
                "    async fn resolve(",
                "    #[allow(unused_variables)]\n    async fn resolve(",
            )
            .replace(
                "::core::unimplemented!()",
                "{ /* business logic */ ::core::unimplemented!() }",
            );
        fs::write(&file.path, &source).unwrap();
        file.synchronize(sdl).unwrap();
        assert_eq!(source, file.read());
    }

    #[test]
    fn collision_rendering_uses_absolute_paths_for_local_names_and_imports() {
        for declaration in [
            "pub struct String;",
            "use ::std::vec::Vec as String;",
            "use ::std::string::String;",
            "use super::*;",
        ] {
            let file = ResolverFile::new();
            fs::write(&file.path, format!(
                "{declaration}\npub struct Query;\nimpl<C: Sync> crate::generated::resolvers::QueryResolver<C> for Query {{}}\n"
            )).unwrap();
            file.synchronize("type Query { label: String! count: Int! }")
                .unwrap();
            assert_eq!(
                field_output(&file.read(), "Query", "label"),
                ":: std :: string :: String",
                "{declaration}"
            );
            if !declaration.contains('*') {
                assert_eq!(field_output(&file.read(), "Query", "count"), "i32");
            }
        }
    }

    #[test]
    fn collision_rendering_respects_impl_generic_names() {
        let file = ResolverFile::new();
        fs::write(&file.path, "pub struct Query;\nimpl<String: Sync> crate::generated::resolvers::QueryResolver<String> for Query {}\n").unwrap();
        file.synchronize("type Query { label: String! }").unwrap();
        assert_eq!(
            field_output(&file.read(), "Query", "label"),
            ":: std :: string :: String"
        );
        assert!(file.read().contains("_context: &String"));
    }

    #[test]
    fn collision_rendering_repairs_paths_when_sdl_adds_shadowing_types() {
        let file = ResolverFile::new();
        file.synchronize("type Query { values: [Int!] }").unwrap();
        let source = file.read().replace(
            "type Output = Option<Vec<i32>>;",
            "type Output = Option</* keep */ Vec<i32>>;",
        );
        fs::write(&file.path, &source).unwrap();
        let sdl = "type Query { values: [Int!] option: Option! vec: Vec! result: Result! sync: Sync! framework: necrassrs! } \
                   type Option { id: ID! } type Vec { id: ID! } type Result { id: ID! } type Sync { id: ID! } type necrassrs { id: ID! }";
        file.synchronize(sdl).unwrap();
        let updated = file.read();
        assert!(updated.contains("::core::option::Option</* keep */ ::std::vec::Vec<i32>>"));
        assert!(
            updated.contains("-> ::core::result::Result<Self::Output, ::necrassrs::ResolverError>")
        );
        assert!(
            updated.contains(
                "impl<C: ::core::marker::Sync> crate::generated::resolvers::QueryResolver"
            )
        );
        file.synchronize(sdl).unwrap();
        assert_eq!(updated, file.read());
    }

    #[test]
    fn collision_rendering_uses_the_destination_module_scope() {
        let file = ResolverFile::new();
        let child = write_external_resolver(&file, "resolvers/fields.rs");
        let source = fs::read_to_string(&child).unwrap();
        fs::write(&child, format!("pub struct Option;\n{source}")).unwrap();
        file.synchronize("type Query { hello: [String!] }").unwrap();
        assert_eq!(
            field_output(&fs::read_to_string(&child).unwrap(), "Query", "hello"),
            ":: core :: option :: Option < Vec < String > >"
        );
        assert!(
            fs::read_to_string(&child)
                .unwrap()
                .contains("/* retain this body */")
        );
    }

    #[test]
    fn readable_names_and_types_keep_keyword_escapes() {
        let file = ResolverFile::new();
        file.synchronize(
            "enum Status { OPEN } type Operator { id: ID! type: String! gen: String! } \
             type Query { operators: [Operator!]! statuses: [Status!] label: String! }",
        )
        .unwrap();
        let source = file.read();
        assert!(source.contains("pub struct Operator;"));
        assert!(source.contains("fields::Operator::id"));
        assert!(source.contains("fields::Operator::r#type"));
        assert!(source.contains("fields::Operator::r#gen"));
        assert!(source.contains("type Output = Vec<self::Operator>;"));
        assert!(source.contains("type Output = Option<Vec<crate::generated::types::Status>>;"));
        assert!(source.contains("type Output = necrassrs::Id;"));
        assert!(source.contains("type Output = String;"));
        assert!(source.contains("-> Result<Self::Output, necrassrs::ResolverError>"));
        file.synchronize(
            "enum Status { OPEN } type Operator { id: ID! type: String! gen: String! } \
             type Query { operators: [Operator!]! statuses: [Status!] label: String! }",
        )
        .unwrap();
        assert_eq!(source, file.read());
    }

    #[test]
    fn rejects_removing_application_written_resolver_before_writing() {
        let file = ResolverFile::new();
        let child = write_external_resolver(&file, "resolvers/fields.rs");
        let child_source = fs::read_to_string(&child).unwrap().replace(
            "/* retain this body */\n        ::core::unimplemented!()",
            "Ok(String::from(\"hello\"))",
        );
        fs::write(&child, &child_source).unwrap();
        let entry_source = file.read();

        let error = file
            .synchronize("type Query { added: String! }")
            .expect_err("application-written resolver was removed");

        assert!(
            matches!(&error, BuildError::ResolverSource { path, .. } if path == &child),
            "{error}"
        );
        assert!(error.to_string().contains("Query.hello"), "{error}");
        assert_eq!(file.read(), entry_source);
        assert_eq!(fs::read_to_string(child).unwrap(), child_source);
    }

    #[test]
    fn rejects_removing_a_comment_modified_stub_before_writing() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! old: String! }")
            .unwrap();
        let mut source = file.read();
        let ast = syn::parse_file(&source).unwrap();
        let implementation = field_implementation_in(&ast.items, "Query", "old").unwrap();
        let range = implementation.span().byte_range();
        let offset = source[range.clone()]
            .find("::core::unimplemented!()")
            .unwrap();
        source.insert_str(
            range.start + offset,
            "/* keep this explanation */\n        ",
        );
        fs::write(&file.path, &source).unwrap();

        let error = file
            .synchronize("type Query { hello: String! }")
            .expect_err("comment-modified stub was removed");

        assert!(error.to_string().contains("Query.old"), "{error}");
        assert_eq!(file.read(), source);
    }

    #[test]
    fn rejects_removing_a_modified_root_marker_before_writing() {
        let marker =
            "impl<C: Sync> crate::generated::resolvers::MutationResolver<C> for self::Mutation {}";
        for replacement in [
            "impl<C: Sync> crate::generated::resolvers::MutationResolver<C> for Mutation {}",
            "impl<C: Sync + Send> crate::generated::resolvers::MutationResolver<C> for self::Mutation {}",
            "impl<C: Sync> crate::generated::resolvers::MutationResolver<C> for self::Mutation { /* keep */ }",
        ] {
            let file = ResolverFile::new();
            file.synchronize(
                "schema { query: Query mutation: Mutation } type Query { hello: String! } type Mutation { update: String! }",
            )
            .unwrap();
            let source = file.read().replace(marker, replacement);
            assert_ne!(source, file.read());
            fs::write(&file.path, &source).unwrap();

            let error = file
                .synchronize("type Query { hello: String! }")
                .expect_err("modified root marker was removed");

            assert!(
                error.to_string().contains(
                    "Cannot remove an application-written root resolver marker automatically"
                ),
                "{error}"
            );
            assert_eq!(file.read(), source);
        }
    }

    #[test]
    fn rejects_removing_an_argument_used_by_a_retained_body() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello(name: String!): String! }")
            .unwrap();
        let source = mutate_resolve(&file.read(), |resolve| {
            resolve.block = syn::parse_quote!({ Ok(_args.name) });
        });
        fs::write(&file.path, &source).unwrap();

        let error = file
            .synchronize("type Query { hello: String! }")
            .expect_err("used resolver argument was removed");

        let message = error.to_string();
        assert!(message.contains("Query.hello"), "{error}");
        assert!(message.contains("name"), "{error}");
        assert_eq!(file.read(), source);
    }

    #[test]
    fn creates_resolvers_and_preserves_unchanged_source() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! }").unwrap();
        let source = file.read();
        let ast = syn::parse_file(&source).unwrap();
        assert!(matches!(&ast.items[0], Item::Struct(item) if item.ident == "Query"));
        assert!(source.contains("fields::Query::hello"));
        assert!(source.contains("async fn resolve"));
        assert!(source.contains("unimplemented"));

        file.synchronize("type Query { hello: String! }").unwrap();
        assert_eq!(file.read(), source);
    }

    #[test]
    fn updates_resolver_moved_to_reachable_external_module_in_place() {
        let file = ResolverFile::new();
        let child = write_external_resolver(&file, "resolvers/fields.rs");
        assert_external_resolver_is_updated(&file, &child, "for super::Query");
    }

    #[test]
    fn updates_resolver_moved_to_reachable_mod_rs_module_in_place() {
        let file = ResolverFile::new();
        let child = write_external_resolver(&file, "resolvers/fields/mod.rs");
        assert_external_resolver_is_updated(&file, &child, "for super::Query");
    }

    #[test]
    fn updates_resolver_moved_to_reachable_nested_external_module_in_place() {
        let file = ResolverFile::new();
        let child = write_nested_external_resolver(&file, "resolvers/group.rs");
        assert_external_resolver_is_updated(&file, &child, "for super::super::Query");
    }

    #[test]
    fn updates_resolver_moved_below_reachable_nested_mod_rs_module_in_place() {
        let file = ResolverFile::new();
        let child = write_nested_external_resolver(&file, "resolvers/group/mod.rs");
        assert_external_resolver_is_updated(&file, &child, "for super::super::Query");
    }

    #[test]
    fn updates_resolver_moved_below_reachable_inline_module_in_place() {
        let file = ResolverFile::new();
        let child = write_external_resolver_below_inline_module(&file);
        assert_external_resolver_is_updated(&file, &child, "for super::super::Query");
    }

    #[test]
    fn updates_resolver_moved_into_inline_module_in_reachable_external_file() {
        let file = ResolverFile::new();
        let child = write_inline_resolver_below_external_module(&file);
        assert_external_resolver_is_updated(&file, &child, "for super::super::Query");
    }

    #[test]
    fn rejects_path_overridden_module_before_writing() {
        let file = ResolverFile::new();
        let conventional = write_external_resolver(&file, "resolvers/fields.rs");
        let entry_source = file
            .read()
            .replace("mod fields;", "#[path = \"custom_fields.rs\"]\nmod fields;");
        fs::write(&file.path, &entry_source).unwrap();
        let conventional_source = fs::read_to_string(&conventional).unwrap();
        let custom = file.directory.join("custom_fields.rs");
        fs::write(&custom, &conventional_source).unwrap();

        let error = file
            .synchronize("type Query { hello: Int! }")
            .expect_err("#[path] module unexpectedly succeeded");

        assert!(error.to_string().contains("#[path]"), "{error}");
        assert_eq!(file.read(), entry_source);
        assert_eq!(
            fs::read_to_string(conventional).unwrap(),
            conventional_source
        );
        assert_eq!(fs::read_to_string(custom).unwrap(), conventional_source);
    }

    #[test]
    fn rejects_cfg_gated_module_before_writing() {
        assert_unsupported_module_syntax_is_rejected("#[cfg(any())]\nmod fields;", "#[cfg]");
    }

    #[test]
    fn ignores_cfg_test_inline_modules_during_discovery() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! }").unwrap();
        let source = format!(
            "{}\n#[cfg(test)]\nmod tests {{\n    resolver_modules!();\n}}\n",
            file.read()
        );
        fs::write(&file.path, &source).unwrap();

        file.synchronize("type Query { hello: String! }").unwrap();

        assert_eq!(file.read(), source);
    }

    #[test]
    fn rejects_cfg_attr_overridden_module_before_writing() {
        assert_unsupported_module_syntax_is_rejected(
            "#[cfg_attr(all(), path = \"custom_fields.rs\")]\nmod fields;",
            "#[cfg_attr]",
        );
    }

    #[test]
    fn rejects_included_resolvers_before_writing() {
        assert_unsupported_module_syntax_is_rejected(
            "include!(\"resolvers/fields.rs\");",
            "include!",
        );
    }

    #[test]
    fn rejects_macro_created_modules_before_writing() {
        assert_unsupported_module_syntax_is_rejected("resolver_modules!();", "resolver_modules!");
    }

    #[test]
    fn reports_both_conventional_paths_for_a_missing_module() {
        let file = ResolverFile::new();
        fs::write(
            &file.path,
            r#"
pub struct Query;

mod fields;

impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for self::Query {}
"#,
        )
        .unwrap();
        let entry_source = file.read();

        let error = file
            .synchronize("type Query { hello: String! }")
            .expect_err("missing resolver module unexpectedly succeeded");
        let message = error.to_string();

        assert!(
            matches!(&error, BuildError::ResolverSource { path, .. } if path == &file.path),
            "{error}"
        );
        assert!(
            message.contains(
                &file
                    .directory
                    .join("resolvers/fields.rs")
                    .display()
                    .to_string()
            ),
            "{error}"
        );
        assert!(
            message.contains(
                &file
                    .directory
                    .join("resolvers/fields/mod.rs")
                    .display()
                    .to_string()
            ),
            "{error}"
        );
        assert_eq!(file.read(), entry_source);
    }

    #[test]
    fn reports_all_duplicate_resolver_locations_before_writing() {
        let file = ResolverFile::new();
        let first = write_external_resolver(&file, "resolvers/fields.rs");
        let entry_source = file
            .read()
            .replace("mod fields;", "mod fields;\nmod other;");
        fs::write(&file.path, &entry_source).unwrap();
        let first_source = fs::read_to_string(&first).unwrap();
        let second = file.directory.join("resolvers/other.rs");
        fs::write(&second, &first_source).unwrap();

        let error = file
            .synchronize("type Query { hello: Int! }")
            .expect_err("duplicate resolver implementations unexpectedly succeeded");
        let message = error.to_string();

        assert!(message.contains(&first.display().to_string()), "{error}");
        assert!(message.contains(&second.display().to_string()), "{error}");
        assert_eq!(file.read(), entry_source);
        assert_eq!(fs::read_to_string(first).unwrap(), first_source);
        assert_eq!(fs::read_to_string(second).unwrap(), first_source);
    }

    #[test]
    fn rediscovers_a_scaffolded_resolver_after_it_is_moved() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! }").unwrap();
        let source = file.read();
        let ast = syn::parse_file(&source).unwrap();
        let implementation = field_implementation_in(&ast.items, "Query", "hello").unwrap();
        let range = implementation.span().byte_range();
        let child_source = source[range.clone()].replace("for self::Query", "for super::Query");
        let mut entry_source = source;
        entry_source.replace_range(range, "mod fields;");
        fs::write(&file.path, &entry_source).unwrap();
        let child = file.directory.join("resolvers/fields.rs");
        fs::create_dir_all(child.parent().unwrap()).unwrap();
        fs::write(&child, child_source).unwrap();

        file.synchronize("type Query { hello: Int! }").unwrap();

        assert_eq!(
            field_output(&fs::read_to_string(&child).unwrap(), "Query", "hello"),
            "i32"
        );
        assert!(
            field_implementation_in(
                &syn::parse_file(&file.read()).unwrap().items,
                "Query",
                "hello"
            )
            .is_none()
        );
        let updated_entry = file.read();
        let updated_child = fs::read_to_string(&child).unwrap();
        file.synchronize("type Query { hello: Int! }").unwrap();
        assert_eq!(file.read(), updated_entry);
        assert_eq!(fs::read_to_string(child).unwrap(), updated_child);
    }

    #[test]
    fn ignores_unreachable_rust_files() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! }").unwrap();
        let unreachable = file.directory.join("unreachable.rs");
        fs::write(&unreachable, "not valid Rust source").unwrap();

        file.synchronize("type Query { hello: Int! }").unwrap();

        assert_eq!(
            fs::read_to_string(unreachable).unwrap(),
            "not valid Rust source"
        );
        assert_eq!(field_output(&file.read(), "Query", "hello"), "i32");
    }

    #[test]
    fn does_not_claim_alias_or_glob_resolver_traits() {
        let cases = [
            (
                "use ::necrassrs::Resolver as FieldResolver;",
                "FieldResolver",
            ),
            ("use ::necrassrs::*;", "Resolver"),
        ];
        for (import, trait_name) in cases {
            let file = ResolverFile::new();
            file.synchronize("type Query { hello: String! }").unwrap();
            let source = file.read();
            let aliased = source.replacen(
                "necrassrs::Resolver<crate::generated::fields",
                &format!("{trait_name}<crate::generated::fields"),
                1,
            );
            assert_ne!(aliased, source);
            fs::write(&file.path, format!("{import}\n{aliased}")).unwrap();

            file.synchronize("type Query { hello: String! }").unwrap();

            let updated = file.read();
            let exact_matches = syn::parse_file(&updated)
                .unwrap()
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Impl(implementation) => field_resolver_coordinate(implementation),
                    _ => None,
                })
                .filter(|coordinate| coordinate == &("Query".to_owned(), "hello".to_owned()))
                .count();
            assert_eq!(exact_matches, 1);
            assert_eq!(updated.matches("type Output").count(), 2);
            file.synchronize("type Query { hello: String! }").unwrap();
            assert_eq!(file.read(), updated);
        }
    }

    #[test]
    fn rejects_ambiguous_conventional_module_paths_before_writing() {
        let file = ResolverFile::new();
        let module_file = write_external_resolver(&file, "resolvers/fields.rs");
        let module_source = fs::read_to_string(&module_file).unwrap();
        let mod_file = file.directory.join("resolvers/fields/mod.rs");
        fs::create_dir_all(mod_file.parent().unwrap()).unwrap();
        fs::write(&mod_file, &module_source).unwrap();
        let entry_source = file.read();

        let error = file
            .synchronize("type Query { hello: Int! }")
            .expect_err("ambiguous module paths unexpectedly succeeded");
        let message = error.to_string();

        assert!(
            message.contains(&module_file.display().to_string()),
            "{error}"
        );
        assert!(message.contains(&mod_file.display().to_string()), "{error}");
        assert_eq!(file.read(), entry_source);
        assert_eq!(fs::read_to_string(module_file).unwrap(), module_source);
        assert_eq!(fs::read_to_string(mod_file).unwrap(), module_source);
    }

    #[test]
    fn updates_multiple_resolver_files_from_one_plan() {
        let file = ResolverFile::new();
        let first = write_external_resolver(&file, "resolvers/first.rs");
        let entry_source = file
            .read()
            .replace("mod fields;", "mod first;\nmod second;");
        fs::write(&file.path, &entry_source).unwrap();
        let first_source = fs::read_to_string(&first).unwrap();
        let second = file.directory.join("resolvers/second.rs");
        let second_source = first_source.replace("hello", "goodbye");
        fs::write(&second, &second_source).unwrap();

        file.synchronize("type Query { hello: Int! goodbye: Int! }")
            .unwrap();

        let first_updated = fs::read_to_string(&first).unwrap();
        let second_updated = fs::read_to_string(&second).unwrap();
        assert_eq!(field_output(&first_updated, "Query", "hello"), "i32");
        assert_eq!(field_output(&second_updated, "Query", "goodbye"), "i32");
        assert!(first_updated.contains("/* retain this body */"));
        assert!(second_updated.contains("/* retain this body */"));
        assert_eq!(file.read(), entry_source);
        file.synchronize("type Query { hello: Int! goodbye: Int! }")
            .unwrap();
        assert_eq!(fs::read_to_string(first).unwrap(), first_updated);
        assert_eq!(fs::read_to_string(second).unwrap(), second_updated);
    }

    #[test]
    fn rolls_back_applied_files_when_commit_fails() {
        let file = ResolverFile::new();
        let first = file.directory.join("first.rs");
        let created = file.directory.join("created.rs");
        let third = file.directory.join("third.rs");
        fs::write(&first, "first original").unwrap();
        fs::write(&third, "third original").unwrap();
        let writes = vec![
            PlannedWrite {
                path: first.clone(),
                original: Some("first original".to_owned()),
                updated: "first updated".to_owned(),
            },
            PlannedWrite {
                path: created.clone(),
                original: None,
                updated: "created".to_owned(),
            },
            PlannedWrite {
                path: third.clone(),
                original: Some("third original".to_owned()),
                updated: "third updated".to_owned(),
            },
        ];
        let mut attempts = 0;

        let error = commit_writes_with(
            writes,
            |path, source| {
                attempts += 1;
                if attempts == 3 {
                    Err(std::io::Error::other("injected commit failure"))
                } else {
                    fs::write(path, source)
                }
            },
            |path| fs::remove_file(path),
        )
        .expect_err("injected commit failure unexpectedly succeeded");

        assert!(error.to_string().contains("injected commit failure"));
        assert_eq!(fs::read_to_string(first).unwrap(), "first original");
        assert!(!created.exists());
        assert_eq!(fs::read_to_string(third).unwrap(), "third original");
    }

    #[test]
    fn detects_source_changes_before_each_replacement() {
        let file = ResolverFile::new();
        let first = file.directory.join("first.rs");
        let second = file.directory.join("second.rs");
        fs::write(&first, "first original").unwrap();
        fs::write(&second, "second original").unwrap();
        let writes = vec![
            PlannedWrite {
                path: first.clone(),
                original: Some("first original".to_owned()),
                updated: "first updated".to_owned(),
            },
            PlannedWrite {
                path: second.clone(),
                original: Some("second original".to_owned()),
                updated: "second updated".to_owned(),
            },
        ];
        let mut attempts = 0;

        let error = commit_writes_with(
            writes,
            |path, source| {
                attempts += 1;
                fs::write(path, source)?;
                if attempts == 1 {
                    fs::write(&second, "second concurrent edit")?;
                }
                Ok(())
            },
            |path| fs::remove_file(path),
        )
        .expect_err("concurrent source change was overwritten");

        assert!(
            error.to_string().contains("changed during synchronization"),
            "{error}"
        );
        assert_eq!(fs::read_to_string(first).unwrap(), "first original");
        assert_eq!(
            fs::read_to_string(second).unwrap(),
            "second concurrent edit"
        );
    }

    #[test]
    fn does_not_rollback_over_a_concurrent_source_change() {
        let file = ResolverFile::new();
        let first = file.directory.join("first.rs");
        let second = file.directory.join("second.rs");
        fs::write(&first, "first original").unwrap();
        fs::write(&second, "second original").unwrap();
        let writes = vec![
            PlannedWrite {
                path: first.clone(),
                original: Some("first original".to_owned()),
                updated: "first updated".to_owned(),
            },
            PlannedWrite {
                path: second.clone(),
                original: Some("second original".to_owned()),
                updated: "second updated".to_owned(),
            },
        ];
        let mut attempts = 0;

        let error = commit_writes_with(
            writes,
            |path, source| {
                attempts += 1;
                if attempts == 2 {
                    fs::write(&first, "first concurrent edit")?;
                    Err(std::io::Error::other("injected commit failure"))
                } else {
                    fs::write(path, source)
                }
            },
            |path| fs::remove_file(path),
        )
        .expect_err("injected commit failure unexpectedly succeeded");

        let message = error.to_string();
        assert!(message.contains("injected commit failure"), "{error}");
        assert!(message.contains("changed during rollback"), "{error}");
        assert_eq!(fs::read_to_string(first).unwrap(), "first concurrent edit");
        assert_eq!(fs::read_to_string(second).unwrap(), "second original");
    }

    #[test]
    fn does_not_replace_destination_when_staging_fails() {
        let file = ResolverFile::new();
        let target = file.directory.join("target.rs");
        fs::write(&target, "original").unwrap();
        let mut staged_path = None;
        let mut replacement_attempted = false;

        let error = replace_file_with(
            &target,
            "updated",
            |staged: &Path, file: &mut fs::File, source: &str| {
                staged_path = Some(staged.to_owned());
                std::io::Write::write_all(file, &source.as_bytes()[..3])?;
                Err(std::io::Error::other("injected staging failure"))
            },
            |_: &Path, _: &Path| {
                replacement_attempted = true;
                Ok(())
            },
        )
        .expect_err("injected staging failure unexpectedly succeeded");

        assert!(error.to_string().contains("injected staging failure"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "original");
        assert!(!replacement_attempted);
        let staged_path = staged_path.expect("no staging path was provided");
        assert_eq!(staged_path.parent(), target.parent());
        assert_ne!(staged_path, target);
        assert!(!staged_path.exists());
    }

    #[test]
    fn reports_commit_and_rollback_failures() {
        let file = ResolverFile::new();
        let first = file.directory.join("first.rs");
        let second = file.directory.join("second.rs");
        let third = file.directory.join("third.rs");
        fs::write(&first, "first original").unwrap();
        fs::write(&second, "second original").unwrap();
        fs::write(&third, "third original").unwrap();
        let writes = vec![
            PlannedWrite {
                path: first.clone(),
                original: Some("first original".to_owned()),
                updated: "first updated".to_owned(),
            },
            PlannedWrite {
                path: second.clone(),
                original: Some("second original".to_owned()),
                updated: "second updated".to_owned(),
            },
            PlannedWrite {
                path: third.clone(),
                original: Some("third original".to_owned()),
                updated: "third updated".to_owned(),
            },
        ];
        let mut attempts = 0;

        let error = commit_writes_with(
            writes,
            |path, source| {
                attempts += 1;
                match attempts {
                    3 => Err(std::io::Error::other("injected commit failure")),
                    4 => Err(std::io::Error::other("injected rollback failure")),
                    _ => fs::write(path, source),
                }
            },
            |path| fs::remove_file(path),
        )
        .expect_err("injected failures unexpectedly succeeded");
        let message = error.to_string();

        assert!(message.contains("injected commit failure"), "{error}");
        assert!(message.contains("injected rollback failure"), "{error}");
        assert_eq!(fs::read_to_string(first).unwrap(), "first original");
        assert_eq!(fs::read_to_string(second).unwrap(), "second updated");
        assert_eq!(fs::read_to_string(third).unwrap(), "third original");
    }

    #[test]
    fn rejects_cycle_forming_module_paths_before_writing() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! }").unwrap();
        let source = format!("#[path = \"resolvers.rs\"]\nmod cycle;\n{}", file.read());
        fs::write(&file.path, &source).unwrap();

        let error = file
            .synchronize("type Query { hello: String! }")
            .expect_err("module cycle unexpectedly succeeded");

        assert!(error.to_string().contains("#[path]"), "{error}");
        assert_eq!(file.read(), source);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_external_modules_before_writing() {
        use std::os::unix::fs::symlink;

        let file = ResolverFile::new();
        let child = write_external_resolver(&file, "resolvers/fields.rs");
        let child_source = fs::read_to_string(&child).unwrap();
        fs::remove_file(&child).unwrap();
        let target = file.directory.join("target.rs");
        fs::write(&target, &child_source).unwrap();
        symlink(&target, &child).unwrap();
        let entry_source = file.read();

        assert!(matches!(
            file.synchronize("type Query { hello: Int! }"),
            Err(BuildError::Io { path, .. }) if path == child
        ));
        assert_eq!(file.read(), entry_source);
        assert_eq!(fs::read_to_string(target).unwrap(), child_source);
    }

    #[test]
    fn updates_resolver_moved_to_reachable_inline_module_in_place() {
        let file = ResolverFile::new();
        fs::write(
            &file.path,
            r#"
pub struct Query;

mod fields {
    impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::Query::hello, C>
        for super::Query
    {
        type Output = ::std::string::String;

        async fn resolve(
            &self,
            _context: &C,
            _args: crate::generated::types::Query::hello::Args,
        ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
            /* retain this body */
            ::core::unimplemented!()
        }
    }
}

impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for self::Query {}
"#,
        )
        .unwrap();

        file.synchronize("type Query { hello: Int! }").unwrap();

        let source = file.read();
        let ast = syn::parse_file(&source).unwrap();
        let inline_items = ast
            .items
            .iter()
            .find_map(|item| match item {
                Item::Mod(module) if module.ident == "fields" => {
                    module.content.as_ref().map(|(_, items)| items)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(field_output_in(inline_items, "Query", "hello"), "i32");
        assert!(source.contains("/* retain this body */"));
        assert!(source.contains("for super::Query"));
        assert!(!ast.items.iter().any(|item| {
            matches!(item, Item::Impl(implementation)
                if field_resolver_coordinate(implementation)
                    == Some(("Query".to_owned(), "hello".to_owned())))
        }));

        file.synchronize("type Query { hello: Int! }").unwrap();
        assert_eq!(file.read(), source);
    }

    #[test]
    fn updates_resolver_moved_to_reachable_nested_inline_module_in_place() {
        let file = ResolverFile::new();
        fs::write(
            &file.path,
            r#"
pub struct Query;

mod group {
    mod fields {
        impl<C: Sync> ::necrassrs::Resolver<crate::generated::fields::Query::hello, C>
            for super::super::Query
        {
            type Output = ::std::string::String;

            async fn resolve(
                &self,
                _context: &C,
                _args: crate::generated::types::Query::hello::Args,
            ) -> ::core::result::Result<Self::Output, ::necrassrs::ResolverError> {
                /* retain this body */
                ::core::unimplemented!()
            }
        }
    }
}

impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for self::Query {}
"#,
        )
        .unwrap();

        file.synchronize("type Query { hello: Int! }").unwrap();

        let source = file.read();
        let ast = syn::parse_file(&source).unwrap();
        let group_items = ast
            .items
            .iter()
            .find_map(|item| match item {
                Item::Mod(module) if module.ident == "group" => {
                    module.content.as_ref().map(|(_, items)| items)
                }
                _ => None,
            })
            .unwrap();
        let inline_items = group_items
            .iter()
            .find_map(|item| match item {
                Item::Mod(module) if module.ident == "fields" => {
                    module.content.as_ref().map(|(_, items)| items)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(field_output_in(inline_items, "Query", "hello"), "i32");
        assert!(source.contains("/* retain this body */"));
        assert!(source.contains("for super::super::Query"));
        assert!(!ast.items.iter().any(|item| {
            matches!(item, Item::Impl(implementation)
                if field_resolver_coordinate(implementation)
                    == Some(("Query".to_owned(), "hello".to_owned())))
        }));

        file.synchronize("type Query { hello: Int! }").unwrap();
        assert_eq!(file.read(), source);
    }

    #[test]
    fn whitespace_only_source_bootstraps_resolvers() {
        let file = ResolverFile::new();
        fs::write(&file.path, "  \n\t\n").unwrap();

        file.synchronize("type Query { hello: String! }").unwrap();

        let source = file.read();
        assert!(source.contains("QueryResolver<C> for self::Query {}"));
        assert!(source.contains("fields::Query::hello"));
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
                "impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for self::Query {}",
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
        let source = file.read().replacen(
            "::core::unimplemented!()",
            "{ /* retain this comment */ Ok(String::from(\"hello\")) }",
            1,
        );
        fs::write(&file.path, &source).unwrap();

        file.synchronize("type Query { hello: String! added: String! }")
            .unwrap();
        let updated = file.read();
        assert!(updated.contains("/* retain this comment */"));
        assert!(updated.contains("fields::Query::added"));
        assert!(!updated.contains("fields::Query::old"));
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
            assert!(updated.contains("fields::Query::hello"));
            assert!(updated.contains("fields::Query::added"));
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
            ("type Query { value: String }", "Option < String >"),
            ("type Query { value: [String!]! }", "Vec < String >"),
            (
                "type Query { value: User! } type User { name: String! }",
                "self :: User",
            ),
            ("type Query { value: Int! }", "i32"),
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
    fn updates_output_when_named_type_kind_changes() {
        let file = ResolverFile::new();
        let enum_schema = "enum Status { OPEN } type Query { value: Status! }";
        let object_schema = "type Status { code: String! } type Query { value: Status! }";

        file.synchronize(enum_schema).unwrap();
        file.synchronize(object_schema).unwrap();
        assert_eq!(
            field_output(&file.read(), "Query", "value"),
            "self :: Status"
        );

        file.synchronize(enum_schema).unwrap();
        assert_eq!(
            field_output(&file.read(), "Query", "value"),
            "crate :: generated :: types :: Status"
        );
    }

    #[test]
    fn rejects_malformed_resolve_methods_without_writing() {
        let file = ResolverFile::new();
        file.synchronize("type Query { hello: String! }").unwrap();
        let source = file.read();
        let cases = [
            (
                mutate_resolve(&source, |resolve| {
                    resolve.sig.ident = syn::parse_quote!(renamed);
                }),
                "Expected one resolve method",
            ),
            (duplicate_resolve(&source), "Expected one resolve method"),
            (
                mutate_resolve(&source, |resolve| {
                    resolve.sig.inputs.pop();
                }),
                "Expected resolve with self, Context, and Args parameters",
            ),
        ];

        for (index, (input, expected)) in cases.into_iter().enumerate() {
            fs::write(&file.path, &input).unwrap();
            let error = file
                .synchronize("type Query { hello: String! added: String! }")
                .expect_err(&format!("case {index} unexpectedly succeeded"));
            assert!(
                error.to_string().contains(expected),
                "case {index}: {error}"
            );
            assert_eq!(file.read(), input);
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
                format!(
                    "{source}\nmod fields {{\n{}\n}}",
                    &source[source.rfind("impl<").unwrap()..]
                ),
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
