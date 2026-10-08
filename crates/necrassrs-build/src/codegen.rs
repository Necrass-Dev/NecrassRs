//! In-memory Rust generation from a validated Apollo schema.
//!
//! Use [`crate::build`] for normal Cargo integration and editable resolver
//! synchronization. This module only generates disposable contract source.

use apollo_compiler::{
    Schema,
    ast::{NamedType, Type},
    parser::SourceSpan,
    schema::ExtendedType,
    validation::Valid,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Returns Rust source containing embedded SDL, argument types, resolver traits,
/// and a schema dispatcher. Does not write files or synchronize user source.
///
/// The output expects a compatible `necrassrs` dependency in the consumer.
/// Field and argument names preserve SDL case, with escaping for Rust identifiers.
/// Fields without arguments receive empty `Args` structs. Default resolver
/// methods panic when their returned futures are polled.
///
/// # Errors
///
/// Returns [`CodegenError`] for unsupported types or mutation and subscription
/// roots. Generated contracts support built-in scalars, enums, ordinary input
/// objects, lists, nullable wrappers, and direct owned non-null Object results with
/// leaf fields. Custom scalars, wrapped, borrowed, or recursive Objects, and
/// abstract output types are not yet supported.
///
/// ```
/// use apollo_compiler::Schema;
/// let schema = Schema::parse_and_validate(
///     "type Query { hello: String! }", "schema.graphql",
/// ).unwrap();
/// let source = necrassrs_build::codegen::generate(&schema)?;
/// assert!(source.contains("QueryResolver"));
/// # Ok::<(), necrassrs_build::codegen::CodegenError>(())
/// ```
pub fn generate(schema: &Valid<Schema>) -> Result<String, CodegenError> {
    let types = generate_types(schema)?;
    let fields = generate_fields(schema);
    let resolvers = generate_resolvers(schema)?;
    let dispatch = generate_dispatch(schema)?;
    let sdl = schema.to_string();

    Ok(quote! {
        pub const SDL: &str = #sdl;

        #types
        #fields
        #resolvers
        #dispatch
    }
    .to_string())
}

fn kosaraju(graph: &[Vec<usize>]) -> Vec<usize> {
    struct Frame {
        node: usize,
        next_neighbor: usize,
    }

    let mut visited = vec![false; graph.len()];
    let mut order = Vec::with_capacity(graph.len());
    let mut stack = Vec::<Frame>::new();

    for start in 0..graph.len() {
        if visited[start] {
            continue;
        }

        visited[start] = true;
        stack.push(Frame {
            node: start,
            next_neighbor: 0,
        });

        while let Some(frame) = stack.last_mut() {
            if frame.next_neighbor < graph[frame.node].len() {
                let neighbor = graph[frame.node][frame.next_neighbor];
                frame.next_neighbor += 1;

                if !visited[neighbor] {
                    visited[neighbor] = true;
                    stack.push(Frame {
                        node: neighbor,
                        next_neighbor: 0,
                    });
                }
            } else {
                let finished = frame.node;
                stack.pop();
                order.push(finished);
            }
        }
    }

    let mut reversed = vec![Vec::new(); graph.len()];

    for (node, neighbors) in graph.iter().enumerate() {
        for &neighbor in neighbors {
            reversed[neighbor].push(node);
        }
    }

    let mut components = vec![usize::MAX; graph.len()];
    let mut component_id = 0;
    let mut pending = Vec::new();

    for &start in order.iter().rev() {
        if components[start] != usize::MAX {
            continue;
        }

        components[start] = component_id;
        pending.push(start);

        while let Some(node) = pending.pop() {
            for &neighbor in &reversed[node] {
                if components[neighbor] == usize::MAX {
                    components[neighbor] = component_id;
                    pending.push(neighbor);
                }
            }
        }

        component_id += 1;
    }

    components
}

fn boxed_input_fields(
    schema: &Valid<Schema>,
) -> std::collections::HashSet<(apollo_compiler::Name, apollo_compiler::Name)> {
    use std::collections::{HashMap, HashSet};

    let inputs: Vec<_> = schema
        .types
        .values()
        .filter_map(|definition| match definition {
            ExtendedType::InputObject(input) => Some(input.as_ref()),
            _ => None,
        })
        .collect();
    let indices: HashMap<_, _> = inputs
        .iter()
        .enumerate()
        .map(|(index, input)| (input.name.clone(), index))
        .collect();
    let mut graph = vec![Vec::new(); inputs.len()];
    for (source, input) in inputs.iter().enumerate() {
        for field in input.fields.values() {
            match field.ty.as_ref() {
                Type::Named(target) | Type::NonNullNamed(target) => {
                    if let Some(&target) = indices.get(target) {
                        graph[source].push(target);
                    }
                }
                Type::List(_) | Type::NonNullList(_) => {}
            }
        }
    }

    let components = kosaraju(&graph);
    let mut boxed = HashSet::new();
    for (source, input) in inputs.iter().enumerate() {
        for field in input.fields.values() {
            if let Type::Named(target) = field.ty.as_ref()
                && let Some(&target) = indices.get(target)
                && components[source] == components[target]
            {
                boxed.insert((input.name.clone(), field.name.clone()));
            }
        }
    }
    boxed
}

fn generate_types(schema: &Valid<Schema>) -> Result<impl quote::ToTokens, CodegenError> {
    let boxed_fields = boxed_input_fields(schema);
    let mut named_types = Vec::new();
    let mut object_modules = Vec::new();

    for (type_name, definition) in &schema.types {
        if type_name.as_str().starts_with("__") {
            continue;
        }

        if let ExtendedType::Enum(enum_type) = definition {
            let name = format_ident!("r#{}", rust_name(type_name.as_str()));
            let variants = enum_type
                .values
                .keys()
                .map(|value| format_ident!("r#{}", rust_name(value.as_str())));

            named_types.push(quote! {
                pub enum #name {
                    #(#variants,)*
                }
            });
            continue;
        }

        if let ExtendedType::InputObject(input_object) = definition {
            let name = format_ident!("r#{}", rust_name(type_name.as_str()));
            let message = format!("Expected a {type_name} input object");

            if input_object.directives.has("oneOf") {
                let (variants, conversions) = input_object
                    .fields
                    .iter()
                    .map(|(field_name, field)| {
                        let variant = format_ident!("r#{}", rust_name(field_name.as_str()));

                        // Only the selected payload's outermost type becomes non-null.
                        let payload_type = field.ty.as_ref().clone().non_null();
                        let mut payload = input_type(schema, &payload_type, &quote! { self })
                            .ok_or_else(|| {
                                CodegenError::new(
                                    format!(
                                        "Unsupported OneOf input type at {type_name}.{field_name}: {}",
                                        field.ty,
                                    ),
                                    schema,
                                    field.ty.location(),
                                )
                            })?;

                        let graphql_name = field_name.as_str();
                        let coordinate = format!("{type_name}.{field_name}");
                        let mut converted = list_item_value(
                            schema,
                            &payload_type,
                            &quote! { value },
                            &coordinate,
                            &quote! { self },
                        )
                        .ok_or_else(|| {
                            CodegenError::new(
                                format!(
                                    "Unsupported OneOf input type at {type_name}.{field_name}: {}",
                                    field.ty,
                                ),
                                schema,
                                field.ty.location(),
                            )
                        })?;

                        if boxed_fields.contains(&(type_name.clone(), field_name.clone())) {
                            payload = quote! { ::std::boxed::Box<#payload> };
                            converted = quote! { ::std::boxed::Box::new(#converted) };
                        }

                        Ok((
                            quote! { #variant(#payload), },
                            quote! { #graphql_name => Ok(Self::#variant(#converted)), },
                        ))
                    })
                    .collect::<Result<(Vec<_>, Vec<_>), CodegenError>>()?;

                let selection_message =
                    format!("Expected exactly one non-null field in {type_name}");
                let unknown_message = format!("Unknown field in {type_name} input object");

                named_types.push(quote! {
                    #[allow(non_camel_case_types)]
                    pub enum #name {
                        #(#variants)*
                    }

                    impl #name {
                        pub(super) fn _from_graphql_value(
                            value: &::necrassrs::JsonValue,
                        ) -> ::core::result::Result<Self, ::necrassrs::ResolverError> {
                            let object = value
                                .as_object()
                                .ok_or_else(|| ::necrassrs::ResolverError::new(#message))?;
                            if object.len() != 1 {
                                return Err(::necrassrs::ResolverError::new(#selection_message));
                            }
                            let (field, value) = object.iter().next().ok_or_else(|| {
                                ::necrassrs::ResolverError::new(#selection_message)
                            })?;
                            if value.is_null() {
                                return Err(::necrassrs::ResolverError::new(#selection_message));
                            }
                            match field.as_str() {
                                #(#conversions)*
                                _ => Err(::necrassrs::ResolverError::new(#unknown_message)),
                            }
                        }
                    }
                });
                continue;
            }

            let (fields, field_values) = input_object
                .fields
                .iter()
                .map(|(field_name, field)| {
                    let graphql_name = field_name.as_str();
                    let member = format_ident!("r#{}", rust_name(graphql_name));
                    let boxed = boxed_fields.contains(&(type_name.clone(), field_name.clone()));
                    let ty = if boxed {
                        field.ty.as_ref().clone().non_null()
                    } else {
                        field.ty.as_ref().clone()
                    };
                    let mut field_type =
                        input_type(schema, &ty, &quote! { self }).ok_or_else(|| {
                            CodegenError::new(
                                format!(
                                    "Unsupported input type at {type_name}.{field_name}: {}",
                                    field.ty,
                                ),
                                schema,
                                field.ty.location(),
                            )
                        })?;
                    let coordinate = format!("{type_name}.{field_name}");
                    let lookup = quote! { object.get(#graphql_name) };
                    let field_value = if boxed {
                        field_type = quote! {
                            ::necrassrs::GraphQLInput<::std::boxed::Box<#field_type>>
                        };
                        list_item_value(
                            schema,
                            &ty,
                            &quote! { value },
                            &coordinate,
                            &quote! { self },
                        )
                        .map(|value| {
                            nullable_input(&lookup, &quote! { ::std::boxed::Box::new(#value) })
                        })
                    } else {
                        input_position_value(schema, &ty, &lookup, &coordinate, &quote! { self })
                    }
                    .ok_or_else(|| {
                        CodegenError::new(
                            format!(
                                "Unsupported input type at {type_name}.{field_name}: {}",
                                field.ty,
                            ),
                            schema,
                            field.ty.location(),
                        )
                    })?;

                    Ok((
                        quote! { pub #member: #field_type, },
                        quote! { #member: #field_value, },
                    ))
                })
                .collect::<Result<(Vec<_>, Vec<_>), CodegenError>>()?;

            named_types.push(quote! {
                pub struct #name {
                    #(#fields)*
                }

                impl #name {
                    pub(super) fn _from_graphql_value(
                        value: &::necrassrs::JsonValue,
                    ) -> ::core::result::Result<Self, ::necrassrs::ResolverError> {
                        let object = value
                            .as_object()
                            .ok_or_else(|| ::necrassrs::ResolverError::new(#message))?;

                        Ok(Self {
                            #(#field_values)*
                        })
                    }
                }
            });
            continue;
        }

        let ExtendedType::Object(object) = definition else {
            continue;
        };
        let object_name = format_ident!("r#{}", rust_name(type_name.as_str()));
        let mut field_modules = Vec::new();

        for (field_name, field) in &object.fields {
            let field_name = field_name.as_str();
            let field_ident = format_ident!("r#{}", rust_name(field_name));
            let (argument_names, argument_types) = field
                .arguments
                .iter()
                .map(|argument| {
                    let argument_type = input_type(
                        schema,
                        argument.ty.as_ref(),
                        &quote! { super::super },
                    )
                    .ok_or_else(|| {
                        CodegenError::new(
                            format!(
                                "Unsupported argument type at {type_name}.{field_name}({}): {}",
                                argument.name, argument.ty,
                            ),
                            schema,
                            argument.ty.location(),
                        )
                    })?;

                    Ok((
                        format_ident!("r#{}", rust_name(argument.name.as_str())),
                        argument_type,
                    ))
                })
                .collect::<Result<(Vec<_>, Vec<_>), CodegenError>>()?;

            field_modules.push(quote! {
                pub mod #field_ident {
                    pub struct Args {
                        #(pub #argument_names: #argument_types,)*
                    }
                }
            });
        }

        object_modules.push(quote! {
            pub mod #object_name {
                #(#field_modules)*
            }
        });
    }

    Ok(quote! {
        #[allow(non_snake_case, non_camel_case_types)]
        pub mod types {
            #(#named_types)*
            #(#object_modules)*
        }
    })
}

fn generate_fields(schema: &Valid<Schema>) -> impl quote::ToTokens {
    let object_modules = schema
        .types
        .iter()
        .filter_map(|(type_name, definition)| {
            if type_name.as_str().starts_with("__") {
                return None;
            }

            let ExtendedType::Object(object) = definition else {
                return None;
            };

            let object_name = format_ident!("r#{}", rust_name(type_name.as_str()));
            let fields = object
                .fields
                .keys()
                .map(|field_name| {
                    let field_name = format_ident!("r#{}", rust_name(field_name.as_str()));

                    quote! {
                        pub struct #field_name;

                        impl ::necrassrs::Field for #field_name {
                            type Args =
                                super::super::types::#object_name::#field_name::Args;
                        }
                    }
                })
                .collect::<Vec<_>>();

            Some(quote! {
                pub mod #object_name {
                    #(#fields)*
                }
            })
        })
        .collect::<Vec<_>>();

    quote! {
        #[allow(dead_code, non_snake_case, non_camel_case_types)]
        pub mod fields {
            #(#object_modules)*
        }
    }
}

fn generate_resolvers(schema: &Valid<Schema>) -> Result<impl quote::ToTokens, CodegenError> {
    let mut resolvers = Vec::new();

    for (type_name, definition) in &schema.types {
        if type_name.as_str().starts_with("__") {
            continue;
        }
        let ExtendedType::Object(object) = definition else {
            continue;
        };

        let object_name = format_ident!("r#{}", rust_name(type_name.as_str()));
        let resolver_name = format_ident!("{}Resolver", rust_name(type_name.as_str()));
        let mut methods = Vec::new();
        for (field_name, field) in &object.fields {
            if direct_object_output(schema, &field.ty).is_some() {
                continue;
            }

            let method_name = format_ident!("r#{}", rust_name(field_name.as_str()));
            let return_type = resolver_return_type(
                schema,
                type_name,
                field_name,
                &field.ty,
                &quote! { super::types },
            )?;
            let message = format!("Resolver {type_name}.{field_name} is not implemented");
            methods.push(quote! {
                fn #method_name<'a>(
                    &'a self,
                    _context: &'a C,
                    _args: super::types::#object_name::#method_name::Args,
                ) -> impl ::core::future::Future<
                    Output = ::core::result::Result<#return_type, ::necrassrs::ResolverError>
                > + ::core::marker::Send + 'a {
                    async { ::core::unimplemented!(#message) }
                }
            });
        }

        resolvers.push(quote! {
            #[allow(dead_code, non_camel_case_types, non_snake_case)]
            pub trait #resolver_name<C> {
                #(#methods)*
            }
        });
    }

    Ok(quote! {
        pub mod resolvers {
            #(#resolvers)*
        }
    })
}

// ponytail: Add nullable and list Object wrappers with their acceptance tests.
fn direct_object_output<'a>(schema: &Schema, ty: &'a Type) -> Option<&'a NamedType> {
    let Type::NonNullNamed(name) = ty else {
        return None;
    };

    matches!(schema.types.get(name), Some(ExtendedType::Object(_))).then_some(name)
}

fn object_value_name(type_name: &str) -> proc_macro2::Ident {
    format_ident!("{}ObjectValue", rust_name(type_name))
}

fn generate_object_dispatchers(
    schema: &Valid<Schema>,
    query_type: &str,
) -> Result<Vec<TokenStream>, CodegenError> {
    schema
        .types
        .iter()
        .filter_map(|(type_name, definition)| {
            if type_name.as_str().starts_with("__") || type_name.as_str() == query_type {
                return None;
            }

            let ExtendedType::Object(object) = definition else {
                return None;
            };

            Some((type_name, object))
        })
        .map(|(type_name, object)| {
            let graphql_type_name = type_name.as_str();
            let object_name = format_ident!("r#{}", rust_name(graphql_type_name));
            let value_name = object_value_name(graphql_type_name);
            let mut bounds = Vec::new();
            let mut branches = Vec::new();

            for (field_name, field) in &object.fields {
                let graphql_field_name = field_name.as_str();
                let field_name = format_ident!("r#{}", rust_name(graphql_field_name));
                let field_type = quote! { super::fields::#object_name::#field_name };
                let return_type = resolver_return_type(
                    schema,
                    graphql_type_name,
                    graphql_field_name,
                    &field.ty,
                    &quote! { super::types },
                )?;
                bounds.push(quote! {
                    T: ::necrassrs::Resolver<#field_type, C, Output = #return_type>
                });

                let arguments = field
                    .arguments
                    .iter()
                    .map(|argument| {
                        let name = argument.name.as_str();
                        let member = format_ident!("r#{}", rust_name(name));
                        let coordinate =
                            format!("{graphql_type_name}.{graphql_field_name}({name})");
                        let value = argument_value(
                            schema,
                            argument.ty.as_ref(),
                            name,
                            &coordinate,
                            &quote! { super::types },
                        )
                        .ok_or_else(|| {
                            CodegenError::new(
                                format!(
                                    "Unsupported argument type at {coordinate}: {}",
                                    argument.ty
                                ),
                                schema,
                                argument.ty.location(),
                            )
                        })?;

                        Ok(quote! { #member: #value, })
                    })
                    .collect::<Result<Vec<_>, CodegenError>>()?;
                let coordinate = format!("{graphql_type_name}.{graphql_field_name}");
                let conversion = output_value(
                    schema,
                    &field.ty,
                    &quote! { value },
                    &coordinate,
                    &quote! { super::types },
                )
                .ok_or_else(|| {
                    CodegenError::new(
                        format!("Unsupported result type at {coordinate}: {}", field.ty),
                        schema,
                        field.ty.inner_named_type().location(),
                    )
                })?;

                branches.push(quote! {
                    #graphql_field_name => {
                        let args = super::types::#object_name::#field_name::Args {
                            #(#arguments)*
                        };
                        let value = <T as ::necrassrs::Resolver<#field_type, C>>::resolve(
                            &self.value,
                            context,
                            args,
                        )
                        .await?;
                        Ok(#conversion)
                    }
                });
            }

            Ok(quote! {
                #[allow(dead_code, non_camel_case_types)]
                struct #value_name<T> {
                    value: T,
                }

                impl<C, T> ::necrassrs::ResolvedObject<C> for #value_name<T>
                where
                    C: ::core::marker::Sync,
                    // ponytail: Add a value lifetime when borrowed Object outputs are required.
                    T: ::core::marker::Send + ::core::marker::Sync + 'static,
                    #(#bounds,)*
                {
                    fn type_name(&self) -> &'static str {
                        #graphql_type_name
                    }

                    fn resolve<'a>(
                        &'a self,
                        context: &'a C,
                        coordinate: ::necrassrs::FieldCoordinate<'a>,
                        _arguments: &'a ::necrassrs::JsonMap,
                    ) -> ::core::pin::Pin<Box<
                        dyn ::core::future::Future<
                            Output = ::core::result::Result<
                                ::necrassrs::ResolvedValue<C>,
                                ::necrassrs::ResolverError,
                            >,
                        > + ::core::marker::Send + 'a,
                    >> {
                        Box::pin(async move {
                            match coordinate.field {
                                #(#branches,)*
                                _ => Err(::necrassrs::ResolverError::new(::std::format!(
                                    "Unknown field {}.{}",
                                    coordinate.parent_type,
                                    coordinate.field,
                                ))),
                            }
                        })
                    }
                }
            })
        })
        .collect()
}

fn generate_dispatch(schema: &Valid<Schema>) -> Result<impl quote::ToTokens, CodegenError> {
    if let Some(root) = schema
        .schema_definition
        .mutation
        .as_ref()
        .or(schema.schema_definition.subscription.as_ref())
    {
        return Err(CodegenError::new(
            "Generated dispatch currently supports query-only schemas",
            schema,
            root.name.location(),
        ));
    }
    let query = schema
        .schema_definition
        .query
        .as_ref()
        .and_then(|name| schema.get_object(name.as_str()))
        .ok_or_else(|| {
            CodegenError::new(
                "A query root object is required",
                schema,
                schema.schema_definition.location(),
            )
        })?;
    let type_name = query.name.as_str();
    let object_name = format_ident!("r#{}", rust_name(type_name));
    let resolver_name = format_ident!("{}Resolver", rust_name(type_name));
    let object_dispatchers = generate_object_dispatchers(schema, type_name)?;
    let mut branches = Vec::new();
    let mut dispatcher_bounds = Vec::new();
    let mut uses_root_resolver = false;

    for (field_name, field) in &query.fields {
        let field_name = field_name.as_str();
        let method_name = format_ident!("r#{}", rust_name(field_name));
        let arguments = field
            .arguments
            .iter()
            .map(|argument| {
                let name = argument.name.as_str();
                let member = format_ident!("r#{}", rust_name(name));
                let coordinate = format!("{type_name}.{field_name}({name})");
                let value = argument_value(
                    schema,
                    argument.ty.as_ref(),
                    name,
                    &coordinate,
                    &quote! { super::types },
                )
                .ok_or_else(|| {
                    CodegenError::new(
                        format!("Unsupported argument type at {coordinate}: {}", argument.ty),
                        schema,
                        argument.ty.location(),
                    )
                })?;

                Ok(quote! {
                    #member: #value,
                })
            })
            .collect::<Result<Vec<_>, CodegenError>>()?;

        let coordinate = format!("{type_name}.{field_name}");
        if let Some(target_type) = direct_object_output(schema, &field.ty) {
            let target_name = target_type.as_str();
            let target_object = schema
                .get_object(target_name)
                .expect("Object result type must exist in a validated schema");
            let target_object_name = format_ident!("r#{}", rust_name(target_name));
            let target_value_name = object_value_name(target_name);
            let field_type = quote! { super::fields::#object_name::#method_name };

            dispatcher_bounds.push(quote! {
                Q: ::necrassrs::Resolver<#field_type, C>
            });
            for (target_field_name, target_field) in &target_object.fields {
                let target_field_name =
                    format_ident!("r#{}", rust_name(target_field_name.as_str()));
                let target_field_type =
                    quote! { super::fields::#target_object_name::#target_field_name };
                let target_return_type = resolver_return_type(
                    schema,
                    target_name,
                    target_field.name.as_str(),
                    &target_field.ty,
                    &quote! { super::types },
                )?;
                dispatcher_bounds.push(quote! {
                    <Q as ::necrassrs::Resolver<#field_type, C>>::Output:
                        ::necrassrs::Resolver<
                            #target_field_type,
                            C,
                            Output = #target_return_type,
                        >
                });
            }
            dispatcher_bounds.push(quote! {
                <Q as ::necrassrs::Resolver<#field_type, C>>::Output:
                    ::core::marker::Send + ::core::marker::Sync + 'static
            });

            branches.push(quote! {
                (#type_name, #field_name) => {
                    let args = super::types::#object_name::#method_name::Args {
                        #(#arguments)*
                    };
                    let value = <Q as ::necrassrs::Resolver<#field_type, C>>::resolve(
                        &self.query,
                        context,
                        args,
                    )
                    .await?;
                    Ok(::necrassrs::ResolvedValue::Object(Box::new(
                        #target_value_name { value },
                    )))
                }
            });
        } else {
            uses_root_resolver = true;
            let conversion = output_value(
                schema,
                &field.ty,
                &quote! { value },
                &coordinate,
                &quote! { super::types },
            )
            .ok_or_else(|| {
                CodegenError::new(
                    format!("Unsupported result type at {coordinate}: {}", field.ty),
                    schema,
                    field.ty.inner_named_type().location(),
                )
            })?;

            branches.push(quote! {
                (#type_name, #field_name) => {
                    let args = super::types::#object_name::#method_name::Args {
                        #(#arguments)*
                    };
                    let value = super::resolvers::#resolver_name::#method_name(
                        &self.query,
                        context,
                        args,
                    )
                    .await?;
                    Ok(#conversion)
                }
            });
        }
    }

    if uses_root_resolver {
        dispatcher_bounds.push(quote! {
            Q: super::resolvers::#resolver_name<C>
        });
    }

    Ok(quote! {
        pub mod dispatch {
            #(#object_dispatchers)*

            pub struct SchemaDispatcher<Q> {
                query: Q,
            }

            impl<Q> SchemaDispatcher<Q> {
                pub fn new(query: Q) -> Self {
                    Self { query }
                }
            }

            impl<C, Q> ::necrassrs::Dispatcher<C> for SchemaDispatcher<Q>
            where
                C: ::core::marker::Sync,
                Q: ::core::marker::Sync,
                #(#dispatcher_bounds,)*
            {
                async fn resolve<'a>(
                    &'a self,
                    context: &'a C,
                    coordinate: ::necrassrs::FieldCoordinate<'a>,
                    _arguments: &'a ::necrassrs::JsonMap,
                ) -> ::core::result::Result<
                    ::necrassrs::ResolvedValue<C>,
                    ::necrassrs::ResolverError,
                > {
                    match (coordinate.parent_type, coordinate.field) {
                        #(#branches,)*
                        _ => Err(::necrassrs::ResolverError::new(::std::format!(
                            "Unknown field {}.{}", coordinate.parent_type, coordinate.field,
                        ))),
                    }
                }
            }
        }
    })
}

pub(crate) fn resolver_return_type(
    schema: &Schema,
    type_name: &str,
    field_name: &str,
    ty: &Type,
    types_path: &TokenStream,
) -> Result<TokenStream, CodegenError> {
    output_type(schema, ty, types_path).ok_or_else(|| {
        CodegenError::new(
            format!("Unsupported return type at {type_name}.{field_name}: {ty}"),
            schema,
            ty.inner_named_type().location(),
        )
    })
}

pub(crate) fn rust_name(name: &str) -> String {
    if name.starts_with('_') || matches!(name, "self" | "Self" | "super" | "crate") {
        format!("_{name}")
    } else {
        name.to_owned()
    }
}

#[derive(Debug, miette::Diagnostic)]
/// An unsupported schema feature, with source location when available.
///
/// Implements [`miette::Diagnostic`] for rendering annotated diagnostics.
pub struct CodegenError {
    message: String,
    #[source_code]
    source: Option<miette::NamedSource<String>>,
    #[label("{message}")]
    span: Option<miette::SourceSpan>,
}

impl CodegenError {
    fn new(message: impl Into<String>, schema: &Schema, location: Option<SourceSpan>) -> Self {
        let (source, span) = location
            .and_then(|location| {
                let source = schema.sources.get(&location.file_id())?;
                Some((
                    miette::NamedSource::new(
                        source.path().to_string_lossy(),
                        source.source_text().to_owned(),
                    ),
                    (location.offset(), location.node_len()).into(),
                ))
            })
            .unzip();
        Self {
            message: message.into(),
            source,
            span,
        }
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CodegenError {}

fn output_type(schema: &Schema, ty: &Type, types_path: &TokenStream) -> Option<TokenStream> {
    match ty {
        Type::NonNullNamed(name) => named_type(schema, name, types_path),
        Type::Named(name) => {
            let inner = named_type(schema, name, types_path)?;
            Some(quote! { ::core::option::Option<#inner> })
        }
        Type::NonNullList(item) => {
            let item = output_type(schema, item, types_path)?;
            Some(quote! { ::std::vec::Vec<#item> })
        }
        Type::List(item) => {
            let item = output_type(schema, item, types_path)?;
            Some(quote! { ::core::option::Option<::std::vec::Vec<#item>> })
        }
    }
}

fn input_type(schema: &Schema, ty: &Type, types_path: &TokenStream) -> Option<TokenStream> {
    match ty {
        Type::NonNullNamed(name) => named_type(schema, name, types_path),
        Type::Named(name) => {
            let inner = named_type(schema, name, types_path)?;
            Some(quote! { ::necrassrs::GraphQLInput<#inner> })
        }
        Type::NonNullList(item) => {
            let item = input_list_item_type(schema, item, types_path)?;
            Some(quote! { ::std::vec::Vec<#item> })
        }
        Type::List(item) => {
            let item = input_list_item_type(schema, item, types_path)?;
            Some(quote! {
                ::necrassrs::GraphQLInput<::std::vec::Vec<#item>>
            })
        }
    }
}

fn input_list_item_type(
    schema: &Schema,
    ty: &Type,
    types_path: &TokenStream,
) -> Option<TokenStream> {
    match ty {
        Type::NonNullNamed(name) => named_type(schema, name, types_path),
        Type::Named(name) => {
            let inner = named_type(schema, name, types_path)?;
            Some(quote! { ::core::option::Option<#inner> })
        }
        Type::NonNullList(item) => {
            let item = input_list_item_type(schema, item, types_path)?;
            Some(quote! { ::std::vec::Vec<#item> })
        }
        Type::List(item) => {
            let item = input_list_item_type(schema, item, types_path)?;
            Some(quote! {
                ::core::option::Option<::std::vec::Vec<#item>>
            })
        }
    }
}

fn argument_value(
    schema: &Schema,
    ty: &Type,
    name: &str,
    coordinate: &str,
    types_path: &TokenStream,
) -> Option<TokenStream> {
    let lookup = quote! {
        _arguments.get(#name)
    };

    input_position_value(schema, ty, &lookup, coordinate, types_path)
}

fn list_value(
    schema: &Schema,
    item: &Type,
    value: &TokenStream,
    coordinate: &str,
    types_path: &TokenStream,
) -> Option<TokenStream> {
    let item = list_item_value(schema, item, &quote! { item }, coordinate, types_path)?;
    let message = format!("Expected a List argument at {coordinate}");
    Some(quote! {
        (#value).as_array()
            .ok_or_else(|| ::necrassrs::ResolverError::new(#message))?
            .iter()
            .map(|item| {
                Ok(#item)
            })
            .collect::<::core::result::Result<
                ::std::vec::Vec<_>,
                ::necrassrs::ResolverError,
            >>()?
    })
}

fn list_item_value(
    schema: &Schema,
    ty: &Type,
    value: &TokenStream,
    coordinate: &str,
    types_path: &TokenStream,
) -> Option<TokenStream> {
    match ty {
        Type::NonNullNamed(type_name) => {
            named_input_value(schema, type_name, value, coordinate, types_path)
        }
        Type::Named(type_name) => {
            let inner = named_input_value(schema, type_name, value, coordinate, types_path)?;
            Some(quote! {
                if (#value).is_null() {
                    None
                } else {
                    Some(#inner)
                }
            })
        }
        Type::NonNullList(item) => list_value(schema, item, value, coordinate, types_path),
        Type::List(item) => {
            let inner = list_value(schema, item, value, coordinate, types_path)?;
            Some(quote! {
                if (#value).is_null() {
                    None
                } else {
                    Some(#inner)
                }
            })
        }
    }
}

fn named_input_value(
    schema: &Schema,
    type_name: &NamedType,
    value: &TokenStream,
    coordinate: &str,
    types_path: &TokenStream,
) -> Option<TokenStream> {
    let message = format!("Expected a {type_name} argument at {coordinate}");
    match type_name.as_str() {
        "Int" => Some(quote! {
            (#value).as_i64()
                .and_then(|value| {
                    <::core::primitive::i32 as ::core::convert::TryFrom<::core::primitive::i64>>::try_from(value).ok()
                })
                .ok_or_else(|| ::necrassrs::ResolverError::new(#message))?
        }),
        "Float" => Some(quote! {
            (#value).as_f64()
                .ok_or_else(|| ::necrassrs::ResolverError::new(#message))?
        }),
        "String" => Some(quote! {
            (#value).as_str()
                .ok_or_else(|| ::necrassrs::ResolverError::new(#message))?
                .to_owned()
        }),
        "Boolean" => Some(quote! {
            (#value).as_bool()
                .ok_or_else(|| ::necrassrs::ResolverError::new(#message))?
        }),
        "ID" => Some(quote! {
            {
                let value = #value;
                value.as_str()
                    .map(::std::borrow::ToOwned::to_owned)
                    .or_else(|| {
                        value.as_i64().map(|value| value.to_string())
                    })
                    .or_else(|| value.as_u64().map(|value| value.to_string()))
                    .map(::necrassrs::Id::from)
                    .ok_or_else(|| ::necrassrs::ResolverError::new(#message))?
            }
        }),
        _ => match schema.types.get(type_name)? {
            ExtendedType::Enum(enum_type) => {
                let enum_name = format_ident!("r#{}", rust_name(type_name.as_str()));
                let (values, variants): (Vec<_>, Vec<_>) = enum_type
                    .values
                    .keys()
                    .map(|value| {
                        (
                            value.as_str(),
                            format_ident!("r#{}", rust_name(value.as_str())),
                        )
                    })
                    .unzip();

                Some(quote! {
                    match (#value).as_str() {
                        #(Some(#values) => #types_path::#enum_name::#variants,)*
                        _ => return Err(::necrassrs::ResolverError::new(#message)),
                    }
                })
            }

            ExtendedType::InputObject(_) => {
                let name = format_ident!("r#{}", rust_name(type_name.as_str()));

                Some(quote! {
                    #types_path::#name::_from_graphql_value(#value)?
                })
            }

            _ => None,
        },
    }
}

fn named_output_value(
    schema: &Schema,
    type_name: &NamedType,
    value: &TokenStream,
    coordinate: &str,
    types_path: &TokenStream,
) -> Option<TokenStream> {
    match type_name.as_str() {
        "String" | "Int" | "Boolean" => Some(quote! {
            ::necrassrs::ResolvedValue::Json(::necrassrs::JsonValue::from(#value))
        }),
        "Float" => {
            let message = format!("Expected a finite Float result at {coordinate}");

            Some(quote! {
                {
                    let value: ::core::primitive::f64 = #value;

                    if !value.is_finite() {
                        ::necrassrs::ResolvedValue::Error(::necrassrs::ResolverError::new(#message))
                    } else {
                        ::necrassrs::ResolvedValue::Json(::necrassrs::JsonValue::from(value))
                    }
                }
            })
        }
        "ID" => Some(quote! {
            ::necrassrs::ResolvedValue::Json(::necrassrs::JsonValue::from((#value).as_str()))
        }),
        _ => match schema.types.get(type_name)? {
            ExtendedType::Enum(enum_type) => {
                let enum_name = format_ident!("r#{}", rust_name(type_name.as_str()));
                let arms = enum_type.values.keys().map(|name| {
                    let graphql_name = name.as_str();
                    let variant = format_ident!("r#{}", rust_name(graphql_name));

                    quote! {
                        #types_path::#enum_name::#variant =>
                            ::necrassrs::ResolvedValue::Json(::necrassrs::JsonValue::from(#graphql_name)),
                    }
                });

                Some(quote! {
                    match #value {
                        #(#arms)*
                    }
                })
            }
            _ => None,
        },
    }
}

fn input_position_value(
    schema: &Schema,
    ty: &Type,
    lookup: &TokenStream,
    coordinate: &str,
    types_path: &TokenStream,
) -> Option<TokenStream> {
    match ty {
        Type::NonNullNamed(type_name) => {
            let message = format!("Expected a {type_name} argument at {coordinate}");

            let value = quote! {
                (#lookup)
                    .ok_or_else(|| {
                        ::necrassrs::ResolverError::new(#message)
                    })?
            };

            named_input_value(schema, type_name, &value, coordinate, types_path)
        }

        Type::Named(type_name) => {
            let value =
                named_input_value(schema, type_name, &quote! { value }, coordinate, types_path)?;

            Some(nullable_input(lookup, &value))
        }

        Type::NonNullList(item) => {
            let message = format!("Expected a List argument at {coordinate}");

            let value = quote! {
                (#lookup)
                    .ok_or_else(|| {
                        ::necrassrs::ResolverError::new(#message)
                    })?
            };

            list_value(schema, item, &value, coordinate, types_path)
        }

        Type::List(item) => {
            let value = list_value(schema, item, &quote! { value }, coordinate, types_path)?;

            Some(nullable_input(lookup, &value))
        }
    }
}

fn nullable_input(lookup: &TokenStream, value: &TokenStream) -> TokenStream {
    quote! {
        match #lookup {
            None => ::necrassrs::GraphQLInput::Undefined,

            Some(value) if value.is_null() => {
                ::necrassrs::GraphQLInput::Null
            }

            Some(value) => {
                ::necrassrs::GraphQLInput::Value(#value)
            }
        }
    }
}

fn named_type(schema: &Schema, name: &NamedType, types_path: &TokenStream) -> Option<TokenStream> {
    match name.as_str() {
        "Int" => Some(quote! { ::core::primitive::i32 }),
        "Float" => Some(quote! { ::core::primitive::f64 }),
        "String" => Some(quote! { ::std::string::String }),
        "Boolean" => Some(quote! { ::core::primitive::bool }),
        "ID" => Some(quote! { ::necrassrs::Id }),
        _ => match schema.types.get(name) {
            Some(ExtendedType::Enum(_)) | Some(ExtendedType::InputObject(_)) => {
                let name = format_ident!("r#{}", rust_name(name.as_str()));
                Some(quote! { #types_path::#name })
            }
            _ => None,
        },
    }
}

fn output_value(
    schema: &Schema,
    ty: &Type,
    value: &TokenStream,
    coordinate: &str,
    types_path: &TokenStream,
) -> Option<TokenStream> {
    match ty {
        Type::NonNullNamed(name) => named_output_value(schema, name, value, coordinate, types_path),
        Type::Named(name) => {
            let inner =
                named_output_value(schema, name, &quote! { value }, coordinate, types_path)?;

            Some(quote! {
                match #value {
                    None => ::necrassrs::ResolvedValue::Json(::necrassrs::JsonValue::Null),
                    Some(value) => #inner,
                }
            })
        }
        Type::NonNullList(item) => output_list_value(schema, item, value, coordinate, types_path),
        Type::List(item) => {
            let inner = output_list_value(schema, item, &quote! { value }, coordinate, types_path)?;

            Some(quote! {
                match #value {
                    None => ::necrassrs::ResolvedValue::Json(::necrassrs::JsonValue::Null),
                    Some(value) => #inner,
                }
            })
        }
    }
}

fn output_list_value(
    schema: &Schema,
    item: &Type,
    value: &TokenStream,
    coordinate: &str,
    types_path: &TokenStream,
) -> Option<TokenStream> {
    let converted_item = output_value(schema, item, &quote! { item }, coordinate, types_path)?;

    Some(quote! {
        {
            let mut items = ::std::vec::Vec::new();

            for item in #value {
                items.push(#converted_item);
            }

            ::necrassrs::ResolvedValue::List(items)
        }
    })
}

#[cfg(test)]
mod test {
    use apollo_compiler::Schema;
    use miette::Diagnostic;
    use quote::{ToTokens, quote};

    fn normalize_type(tokens: proc_macro2::TokenStream) -> String {
        syn::parse2::<syn::Type>(tokens)
            .expect("generated type must be valid Rust")
            .to_token_stream()
            .to_string()
    }

    #[test]
    fn boxed_input_fields_ignore_non_recursive_references() {
        assert_boxed_input_fields("type Query { next: Query }", &[]);
        assert_boxed_input_fields(
            r#"
                enum Status { OPEN CLOSED }
                input Leaf { name: String! }
                input Root { child: Leaf! optional: Leaf status: Status count: Int }
                type Query { inspect(input: Root): String }
            "#,
            &[],
        );
    }

    #[test]
    fn boxed_input_fields_include_all_nullable_self_edges_regardless_of_defaults() {
        assert_boxed_input_fields(
            r#"
                input Node {
                    next: Node = { next: null }
                    other: Node
                    name: String
                }
                type Query { inspect(input: Node): String }
            "#,
            &[("Node", "next"), ("Node", "other")],
        );
    }

    #[test]
    fn boxed_input_fields_use_non_null_edges_for_component_analysis() {
        assert_boxed_input_fields(
            r#"
                input A { b: B! }
                input B { c: C! }
                input C { a: A }
                type Query { inspect(input: A): String }
            "#,
            &[("C", "a")],
        );
    }

    #[test]
    fn boxed_input_fields_exclude_edges_between_distinct_components() {
        assert_boxed_input_fields(
            r#"
                input A { b: B otherB: B d: D }
                input B { a: A }
                input C { d: D! }
                input D { c: C }
                type Query { inspect(input: A): String }
            "#,
            &[("A", "b"), ("A", "otherB"), ("B", "a"), ("D", "c")],
        );
    }

    #[test]
    fn boxed_input_fields_exclude_all_list_edges_from_component_analysis() {
        assert_boxed_input_fields(
            r#"
                input Node {
                    nullable: [Node]
                    requiredItems: [Node!]
                    required: [Node]!
                    requiredBoth: [Node!]!
                    nested: [[Node!]!]!
                }
                input A { bs: [B]! }
                input B { a: A }
                input C { d: D! }
                input D { cs: [[C!]!]! }
                type Query { inspect(node: Node, a: A, c: C): String }
            "#,
            &[],
        );
    }

    #[test]
    fn boxed_input_fields_use_original_one_of_member_nullability() {
        assert_boxed_input_fields(
            r#"
                input Choice @oneOf {
                    number: Int
                    next: Choice
                    holder: Holder
                    children: [Choice]
                }
                input Holder { choice: Choice! }
                type Query { inspect(input: Choice): String }
            "#,
            &[("Choice", "next"), ("Choice", "holder")],
        );
    }

    #[test]
    fn boxed_input_fields_preserve_graphql_names() {
        assert_boxed_input_fields(
            r#"
                input self { type: self Type: self _type: self }
                type Query { inspect(input: self): String }
            "#,
            &[("self", "type"), ("self", "Type"), ("self", "_type")],
        );
    }

    fn assert_boxed_input_fields(source: &str, expected: &[(&str, &str)]) {
        let schema = Schema::parse_and_validate(source, "schema.graphql")
            .expect("boxing analysis fixtures must be valid schemas");
        let mut actual: Vec<_> = super::boxed_input_fields(&schema)
            .into_iter()
            .map(|(ty, field)| (ty.as_str().to_owned(), field.as_str().to_owned()))
            .collect();
        let mut expected: Vec<_> = expected
            .iter()
            .map(|(ty, field)| ((*ty).to_owned(), (*field).to_owned()))
            .collect();
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected, "{source}");
    }

    #[test]
    fn kosaraju_partitions_directed_graphs() {
        let cases = [
            ("empty", vec![], vec![]),
            (
                "isolated vertices",
                vec![vec![], vec![], vec![]],
                vec![vec![0], vec![1], vec![2]],
            ),
            ("self loops", vec![vec![0], vec![1]], vec![vec![0], vec![1]]),
            (
                "one-way chain",
                vec![vec![1], vec![2], vec![]],
                vec![vec![0], vec![1], vec![2]],
            ),
            (
                "cycle",
                vec![vec![1], vec![2], vec![0]],
                vec![vec![0, 1, 2]],
            ),
            (
                "cycles joined by one-way edges",
                vec![vec![1], vec![0, 2], vec![3], vec![2, 4], vec![]],
                vec![vec![0, 1], vec![2, 3], vec![4]],
            ),
            (
                "disconnected cycles and duplicate edges",
                vec![vec![1, 1], vec![0], vec![3], vec![2], vec![]],
                vec![vec![0, 1], vec![2, 3], vec![4]],
            ),
            (
                "sibling edges require DFS completion order",
                vec![vec![1, 2], vec![2], vec![]],
                vec![vec![0], vec![1], vec![2]],
            ),
            (
                "connected cycles form one component",
                vec![vec![1], vec![0, 2], vec![3], vec![2, 0]],
                vec![vec![0, 1, 2, 3]],
            ),
        ];
        for (case, graph, expected) in cases {
            assert_scc_partition(case, &graph, expected);
        }
    }

    #[test]
    fn kosaraju_partition_is_independent_of_vertex_and_neighbor_order() {
        let graph = vec![vec![1, 2], vec![0, 3], vec![3], vec![2, 4], vec![]];
        assert_scc_partition("original", &graph, vec![vec![0, 1], vec![2, 3], vec![4]]);
        let mut reordered = graph.clone();
        for neighbors in &mut reordered {
            neighbors.reverse();
        }
        assert_scc_partition(
            "reversed neighbors",
            &reordered,
            vec![vec![0, 1], vec![2, 3], vec![4]],
        );

        let permutation = [2, 4, 0, 3, 1];
        let mut renamed = vec![Vec::new(); graph.len()];
        for (node, neighbors) in graph.iter().enumerate() {
            renamed[permutation[node]] = neighbors
                .iter()
                .map(|&neighbor| permutation[neighbor])
                .collect();
        }
        assert_scc_partition(
            "renamed vertices",
            &renamed,
            vec![vec![2, 4], vec![0, 3], vec![1]],
        );
    }

    fn assert_scc_partition(case: &str, graph: &[Vec<usize>], mut expected: Vec<Vec<usize>>) {
        let components = super::kosaraju(graph);
        assert_eq!(
            components.len(),
            graph.len(),
            "{case}: every vertex needs a component"
        );
        let mut groups = std::collections::BTreeMap::<usize, Vec<usize>>::new();
        for (node, component) in components.into_iter().enumerate() {
            groups.entry(component).or_default().push(node);
        }
        let mut actual: Vec<_> = groups.into_values().collect();
        actual.sort_unstable();
        for group in &mut expected {
            group.sort_unstable();
        }
        expected.sort_unstable();
        assert_eq!(actual, expected, "{case}");
    }

    #[test]
    fn kosaraju_handles_deep_graphs_without_recursion() {
        const CHILD_ENV: &str = "NECRASSRS_KOSARAJU_DEEP_GRAPH_CHILD";
        if std::env::var_os(CHILD_ENV).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "codegen::test::kosaraju_handles_deep_graphs_without_recursion",
                    "--nocapture",
                ])
                .env(CHILD_ENV, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "deep graph analysis failed ({})\nstdout:\n{}\nstderr:\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }

        const NODES: usize = 100_000;
        let mut graph: Vec<Vec<usize>> = (0..NODES)
            .map(|node| {
                if node + 1 < NODES {
                    vec![node + 1]
                } else {
                    vec![]
                }
            })
            .collect();
        let mut components = super::kosaraju(&graph);
        assert_eq!(components.len(), NODES);
        components.sort_unstable();
        components.dedup();
        assert_eq!(components.len(), NODES, "a chain has no shared components");

        graph[NODES - 1].push(0);
        let components = super::kosaraju(&graph);
        assert_eq!(components.len(), NODES);
        assert!(
            components
                .iter()
                .all(|component| *component == components[0]),
            "a closed chain is one component"
        );
    }

    #[test]
    fn codegen_error_preserves_argument_type_source_and_span() {
        let source = "# Source: café\nextend type Query { lookup(at: Timestamp!): String! }";
        let schema = Schema::builder()
            .parse(
                "scalar Timestamp type Query { hello: String! }",
                "schema.graphql",
            )
            .parse(source, "lookup.graphql")
            .build()
            .expect("the test schema must build")
            .validate()
            .expect("the test schema must be valid");

        let error = super::generate(&schema).expect_err("the fixture requires a codegen rejection");
        assert!(error.to_string().contains("Query.lookup(at)"));
        let labels: Vec<_> = error
            .labels()
            .expect("the error must have labels")
            .collect();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].offset(), source.find("Timestamp!").unwrap());
        assert_eq!(labels[0].len(), "Timestamp!".len());
        let contents = error
            .source_code()
            .expect("the error must retain its source")
            .read_span(&(0, source.len()).into(), 0, 0)
            .expect("the original source must be readable");
        assert_eq!(contents.name(), Some("lookup.graphql"));
        assert_eq!(contents.data(), source.as_bytes());
    }

    #[test]
    fn codegen_error_without_type_location_has_no_fabricated_source_or_span() {
        let mut schema = Schema::parse(
            "scalar Timestamp type Query { lookup(at: Timestamp!): String! }",
            "schema.graphql",
        )
        .expect("the test schema must parse");
        let apollo_compiler::schema::ExtendedType::Object(query) =
            schema.types.get_mut("Query").unwrap()
        else {
            panic!("the query root must be an object");
        };
        let field = query
            .make_mut()
            .fields
            .get_mut("lookup")
            .unwrap()
            .make_mut();
        field.arguments[0].make_mut().ty =
            apollo_compiler::Node::new(apollo_compiler::ty!(Timestamp!));
        let schema = schema.validate().expect("the test schema must be valid");

        let error = super::generate(&schema).expect_err("the fixture requires a codegen rejection");
        assert!(error.to_string().contains("Query.lookup(at)"));
        assert!(error.source_code().is_none());
        assert_eq!(error.labels().into_iter().flatten().count(), 0);
    }

    #[test]
    fn generated_resolver_accepts_borrowed_context_and_returns_send_future() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! ping: String! }",
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("generation must succeed");
        let consumer = r#"
            use resolvers::QueryResolver;

            pub struct Query {
                greeting: String,
            }

            pub struct Context<'a> {
                suffix: &'a str,
            }

            impl<'ctx> QueryResolver<Context<'ctx>> for Query {
                async fn hello<'a>(
                    &'a self,
                    context: &'a Context<'ctx>,
                    args: types::Query::hello::Args,
                ) -> Result<String, necrassrs::ResolverError> {
                    std::future::ready(()).await;
                    Ok(format!("{} {}{}", self.greeting, args.name, context.suffix))
                }
            }

            pub fn check_contract<'a, C: 'a, R: QueryResolver<C> + 'a>(
                resolver: &'a R,
                context: &'a C,
                args: types::Query::hello::Args,
            ) -> impl Future<Output = Result<String, necrassrs::ResolverError>> + Send + 'a {
                resolver.hello(context, args)
            }

            pub fn check() {
                let query = Query { greeting: String::from("Hello") };
                let suffix = String::from("!");
                let context = Context { suffix: &suffix };
                let args = types::Query::hello::Args { name: String::from("Sheri") };
                let future = check_contract(&query, &context, args);
                drop(future);
                let unimplemented = query.ping(&context, types::Query::ping::Args {});
                drop(unimplemented);
            }
        "#;

        assert_consumer_compiles(&generated, consumer);
    }

    #[test]
    fn generated_dispatch_executes_hello_with_embedded_sdl() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("generation must succeed");
        let consumer = r#"
            use generated::{resolvers::QueryResolver, types};

            struct Query;
            struct Context<'a> { greeting: &'a str }

            impl<'ctx> QueryResolver<Context<'ctx>> for Query {
                async fn hello<'a>(
                    &'a self,
                    context: &'a Context<'ctx>,
                    args: types::Query::hello::Args,
                ) -> Result<String, necrassrs::ResolverError> {
                    Ok(format!("{}, {}", context.greeting, args.name))
                }
            }

            fn main() {
                let schema = necrassrs::Schema::parse_and_validate(
                    generated::SDL, "schema.graphql",
                ).unwrap();
                let dispatcher = generated::dispatch::SchemaDispatcher::new(Query);
                let greeting = String::from("Hello");
                let context = Context { greeting: &greeting };
                let request = necrassrs::Request::new("{ hello(name: \"Sheri\") }");
                let future = necrassrs::execute(&schema, &request, &dispatcher, &context);
                fn assert_send<T: Send>(_: &T) {}
                assert_send(&future);
                let response = futures::executor::block_on(future);

                assert_eq!(
                    serde_json::to_value(response).unwrap(),
                    serde_json::json!({ "data": { "hello": "Hello, Sheri" } }),
                );
            }
        "#;

        assert_consumer(&format!("mod generated {{ {generated} }}"), consumer, true);
    }

    #[test]
    fn generated_consumer_resolves_a_nested_field_against_its_parent_object() {
        let schema = Schema::parse_and_validate(
            "type Query { viewer: User! } type User { name: String! }",
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("composite generation must succeed");
        let consumer = r#"
            use generated::fields;
            use necrassrs::{Field, Resolver};

            struct Query;
            struct User { name: String }

            impl<C: Sync> Resolver<fields::Query::viewer, C> for Query {
                type Output = User;

                async fn resolve(
                    &self,
                    _context: &C,
                    _args: <fields::Query::viewer as Field>::Args,
                ) -> Result<Self::Output, necrassrs::ResolverError> {
                    Ok(User { name: String::from("Sheri") })
                }
            }

            impl<C: Sync> Resolver<fields::User::name, C> for User {
                type Output = String;

                async fn resolve(
                    &self,
                    _context: &C,
                    _args: <fields::User::name as Field>::Args,
                ) -> Result<Self::Output, necrassrs::ResolverError> {
                    Ok(self.name.clone())
                }
            }

            fn main() {
                let schema = necrassrs::Schema::parse_and_validate(
                    generated::SDL, "schema.graphql",
                ).unwrap();
                let dispatcher = generated::dispatch::SchemaDispatcher::new(Query);
                let request = necrassrs::Request::new("{ viewer { name } }");
                let future = necrassrs::execute(&schema, &request, &dispatcher, &());
                fn assert_send<T: Send>(_: &T) {}
                assert_send(&future);
                let response = futures::executor::block_on(future);

                assert_eq!(
                    serde_json::to_value(response).unwrap(),
                    serde_json::json!({ "data": { "viewer": { "name": "Sheri" } } }),
                );
            }
        "#;

        assert_consumer(&format!("mod generated {{ {generated} }}"), consumer, true);
    }

    #[test]
    fn generated_consumer_resolves_object_list_items_against_each_parent() {
        let schema = Schema::parse_and_validate(
            "type Query { users: [User!]! } type User { greeting(prefix: String!): String! }",
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("Object list generation must succeed");
        let consumer = r#"
            use generated::fields;
            use necrassrs::{Field, Resolver};

            struct Query;
            struct User { name: String }
            struct Context<'a> { punctuation: &'a str }

            impl<C: Sync> Resolver<fields::Query::users, C> for Query {
                type Output = Vec<User>;

                async fn resolve(
                    &self,
                    _context: &C,
                    _args: <fields::Query::users as Field>::Args,
                ) -> Result<Self::Output, necrassrs::ResolverError> {
                    Ok(vec![
                        User { name: String::from("Sheri") },
                        User { name: String::from("Riri") },
                    ])
                }
            }

            impl<'ctx> Resolver<fields::User::greeting, Context<'ctx>> for User {
                type Output = String;

                async fn resolve(
                    &self,
                    context: &Context<'ctx>,
                    args: <fields::User::greeting as Field>::Args,
                ) -> Result<Self::Output, necrassrs::ResolverError> {
                    Ok(format!("{}, {}{}", args.prefix, self.name, context.punctuation))
                }
            }

            fn main() {
                let schema = necrassrs::Schema::parse_and_validate(
                    generated::SDL, "schema.graphql",
                ).unwrap();
                let dispatcher = generated::dispatch::SchemaDispatcher::new(Query);
                let punctuation = String::from("!");
                let context = Context { punctuation: &punctuation };
                let request = necrassrs::Request::new(
                    "{ users { greeting(prefix: \"Hello\") } }",
                );
                let future = necrassrs::execute(&schema, &request, &dispatcher, &context);
                fn assert_send<T: Send>(_: &T) {}
                assert_send(&future);
                let response = futures::executor::block_on(future);

                assert_eq!(
                    serde_json::to_value(response).unwrap(),
                    serde_json::json!({
                        "data": {
                            "users": [
                                { "greeting": "Hello, Sheri!" },
                                { "greeting": "Hello, Riri!" },
                            ],
                        },
                    }),
                );
            }
        "#;

        assert_consumer(&format!("mod generated {{ {generated} }}"), consumer, true);
    }

    #[test]
    fn one_of_members_cannot_shadow_input_conversion() {
        let schema = Schema::parse_and_validate(
            "input Choice @oneOf { from_graphql_value: String _from_graphql_value: String } type Query { echo(value: Choice!): String! }",
            "schema.graphql",
        ).unwrap();
        let generated = super::generate(&schema).unwrap();
        assert_consumer(
            &generated,
            r#"
                struct Query;
                impl resolvers::QueryResolver<()> for Query {
                    async fn echo(&self, _: &(), args: types::Query::echo::Args)
                        -> Result<String, necrassrs::ResolverError> {
                        Ok(match args.value {
                            types::Choice::from_graphql_value(value)
                            | types::Choice::__from_graphql_value(value) => value,
                        })
                    }
                }
                fn main() {
                    let schema = necrassrs::Schema::parse_and_validate(SDL, "schema.graphql").unwrap();
                    let dispatcher = dispatch::SchemaDispatcher::new(Query);
                    for name in ["from_graphql_value", "_from_graphql_value"] {
                        let request = necrassrs::Request::new(format!(
                            "{{ echo(value: {{{name}: \"Sheri\"}}) }}",
                        ));
                        let response = futures::executor::block_on(necrassrs::execute(
                            &schema, &request, &dispatcher, &(),
                        ));
                        assert_eq!(serde_json::to_value(response).unwrap(),
                            serde_json::json!({"data": {"echo": "Sheri"}}));
                    }
                }
            "#,
            true,
        );
    }

    #[test]
    fn scalar_primitives_cannot_be_shadowed_by_sdl_type_names() {
        let schema = Schema::parse_and_validate(
            r#"
                input i32 { name: String }
                input f64 { name: String }
                enum bool { YES NO }
                input Filter { number: Int! decimal: Float! flag: Boolean! }
                input Choice @oneOf { number: Int decimal: Float flag: Boolean }
                type Query { inspect(value: Filter!, choice: Choice!, shadow: i32, floating: f64, kind: bool): [String!]! }
            "#,
            "schema.graphql",
        )
        .unwrap();
        let generated = super::generate(&schema).unwrap();
        assert_consumer(
            &format!("#[allow(non_camel_case_types)] pub mod generated {{ {generated} }}"),
            r#"
                use generated::{types, resolvers::QueryResolver};
                struct Query;
                impl QueryResolver<()> for Query {
                    async fn inspect(&self, _: &(), args: types::Query::inspect::Args)
                        -> Result<Vec<String>, necrassrs::ResolverError> {
                        let number: ::core::primitive::i32 = args.value.number;
                        let decimal: ::core::primitive::f64 = args.value.decimal;
                        let flag: ::core::primitive::bool = args.value.flag;
                        let choice = match args.choice {
                            types::Choice::number(value) => value.to_string(),
                            types::Choice::decimal(value) => value.to_string(),
                            types::Choice::flag(value) => value.to_string(),
                        };
                        Ok(vec![number.to_string(), decimal.to_string(), flag.to_string(), choice])
                    }
                }
                fn main() {
                    let schema = necrassrs::Schema::parse_and_validate(generated::SDL, "schema.graphql").unwrap();
                    let dispatcher = generated::dispatch::SchemaDispatcher::new(Query);
                    for (member, expected) in [("number: 7", "7"), ("decimal: 1.5", "1.5"), ("flag: true", "true")] {
                        let request = necrassrs::Request::new(format!(
                            "{{ inspect(value: {{number: 2, decimal: 2.5, flag: false}}, choice: {{{member}}}) }}",
                        ));
                        let response = futures::executor::block_on(necrassrs::execute(
                            &schema, &request, &dispatcher, &(),
                        ));
                        assert_eq!(serde_json::to_value(response).unwrap(),
                            serde_json::json!({"data": {"inspect": ["2", "2.5", "false", expected]}}));
                    }
                }
            "#,
            true,
        );
    }

    #[test]
    fn generated_id_inputs_preserve_unbounded_integer_literals_and_defaults() {
        let schema = Schema::parse_and_validate(
            r#"
                input Holder {
                    id: ID! = 18446744073709551617
                    ids: [ID!]! = -18446744073709551617
                }
                type Query {
                    echo(value: ID! = 18446744073709551617): ID!
                    inspect(value: Holder! = {}): [ID!]!
                }
            "#,
            "schema.graphql",
        )
        .unwrap();
        let generated = super::generate(&schema).unwrap();
        assert_consumer(
            &generated,
            r#"
                struct Query;
                impl resolvers::QueryResolver<()> for Query {
                    async fn echo(&self, _: &(), args: types::Query::echo::Args)
                        -> Result<necrassrs::Id, necrassrs::ResolverError> { Ok(args.value) }
                    async fn inspect(&self, _: &(), args: types::Query::inspect::Args)
                        -> Result<Vec<necrassrs::Id>, necrassrs::ResolverError> {
                        let mut ids = vec![args.value.id];
                        ids.extend(args.value.ids);
                        Ok(ids)
                    }
                }
                fn main() {
                    let schema = necrassrs::Schema::parse_and_validate(SDL, "schema.graphql").unwrap();
                    let dispatcher = dispatch::SchemaDispatcher::new(Query);
                    let run = |document: &str, variables: serde_json::Value| {
                        let request = necrassrs::Request::new(document)
                            .with_variables(serde_json::from_value(variables).unwrap());
                        serde_json::to_value(futures::executor::block_on(necrassrs::execute(
                            &schema, &request, &dispatcher, &(),
                        ))).unwrap()
                    };
                    for id in ["18446744073709551617".to_owned(), "-18446744073709551617".to_owned(), "9".repeat(400)] {
                        for document in [
                            format!("{{ echo(value: {id}) }}"),
                            format!("query($id: ID! = {id}) {{ echo(value: $id) }}"),
                        ] {
                            assert_eq!(run(&document, serde_json::json!({})),
                                serde_json::json!({"data": {"echo": id}}), "{document}");
                        }
                        let document = format!("{{ inspect(value: {{id: {id}, ids: {id}}}) }}");
                        assert_eq!(run(&document, serde_json::json!({})),
                            serde_json::json!({"data": {"inspect": [id, id]}}));
                        let document = format!("query($v: Holder! = {{id: {id}, ids: [{id}]}}) {{inspect(value: $v)}}");
                        assert_eq!(run(&document, serde_json::json!({})),
                            serde_json::json!({"data": {"inspect": [id, id]}}));
                    }
                    for (document, variables) in [
                        ("{ inspect }", serde_json::json!({})),
                        ("{ inspect(value: {}) }", serde_json::json!({})),
                        ("query($v: Holder! = {}) {inspect(value: $v)}", serde_json::json!({})),
                        ("query($v: Holder!) {inspect(value: $v)}", serde_json::json!({"v": {}})),
                    ] {
                        assert_eq!(run(document, variables), serde_json::json!({"data": {
                            "inspect": ["18446744073709551617", "-18446744073709551617"]
                        }}), "{document}");
                    }
                    assert_eq!(run("{ echo }", serde_json::json!({})),
                        serde_json::json!({"data": {"echo": "18446744073709551617"}}));
                }
            "#,
            true,
        );
    }

    #[test]
    fn generated_sdl_names_compile_with_naming_warnings_denied() {
        let schema = Schema::parse_and_validate(
            r#"
                enum status { open in_progress type self _self }
                input filter { status: status! next: filter }
                input choice @oneOf { filter: filter status: status }
                type Query { echo(value: filter!, choice: choice!): status! }
            "#,
            "schema.graphql",
        )
        .unwrap();
        let generated = super::generate(&schema).unwrap();
        assert_consumer(
            &generated,
            r#"
                struct Query;
                impl resolvers::QueryResolver<()> for Query {
                    async fn echo(&self, _: &(), args: types::Query::echo::Args)
                        -> Result<types::status, necrassrs::ResolverError> {
                        Ok(match args.choice {
                            types::choice::filter(value) => value.status,
                            types::choice::status(value) => value,
                        })
                    }
                }
                fn main() {
                    let _: types::status = types::status::r#type;
                    let _: types::status = types::status::_self;
                    let _: types::status = types::status::__self;
                    let _: necrassrs::GraphQLInput<Box<types::filter>> =
                        types::filter { status: types::status::open, next: necrassrs::GraphQLInput::Undefined }.next;
                    let schema = necrassrs::Schema::parse_and_validate(SDL, "schema.graphql").unwrap();
                    let dispatcher = dispatch::SchemaDispatcher::new(Query);
                    for name in ["open", "in_progress", "type", "self", "_self"] {
                        let request = necrassrs::Request::new(format!(
                            "{{ echo(value: {{status: open}}, choice: {{status: {name}}}) }}",
                        ));
                        let response = futures::executor::block_on(necrassrs::execute(
                            &schema, &request, &dispatcher, &(),
                        ));
                        assert_eq!(serde_json::to_value(response).unwrap(),
                            serde_json::json!({"data": {"echo": name}}));
                    }
                }
            "#,
            true,
        );
    }

    #[test]
    fn generated_consumer_executes_leaf_boundaries_and_variable_numbers() {
        let schema = Schema::parse_and_validate(
            "type Query { integer(value: Int!): Int! float(value: Float!): Float! text(value: String!): String! boolean(value: Boolean!): Boolean! id(value: ID!): ID! nullable(value: [Int]): [Int] required(value: [Int]!): [Int]! nitems(value: [Int!]): [Int!] strict(value: [Int!]!): [Int!]! }",
            "schema.graphql",
        ).unwrap();
        let generated = super::generate(&schema).unwrap();
        let consumer = r##"
            use generated::{resolvers::QueryResolver, types};
            struct Query;
            macro_rules! echo {
                ($method:ident, $ty:ty) => {
                    async fn $method(&self, _: &(), args: types::Query::$method::Args)
                        -> Result<$ty, necrassrs::ResolverError> { Ok(args.value) }
                };
            }
            impl QueryResolver<()> for Query {
                echo!(integer, i32);
                echo!(float, f64);
                echo!(text, String);
                echo!(boolean, bool);
                echo!(id, necrassrs::Id);
                echo!(required, Vec<Option<i32>>);
                echo!(strict, Vec<i32>);
                async fn nullable(&self, _: &(), args: types::Query::nullable::Args)
                    -> Result<Option<Vec<Option<i32>>>, necrassrs::ResolverError> {
                    Ok(match args.value {
                        necrassrs::GraphQLInput::Value(value) => Some(value),
                        necrassrs::GraphQLInput::Undefined | necrassrs::GraphQLInput::Null => None,
                    })
                }
                async fn nitems(&self, _: &(), args: types::Query::nitems::Args)
                    -> Result<Option<Vec<i32>>, necrassrs::ResolverError> {
                    Ok(match args.value {
                        necrassrs::GraphQLInput::Value(value) => Some(value),
                        necrassrs::GraphQLInput::Undefined | necrassrs::GraphQLInput::Null => None,
                    })
                }
            }
            fn main() {
                let schema = necrassrs::Schema::parse_and_validate(generated::SDL, "schema.graphql").unwrap();
                let dispatcher = generated::dispatch::SchemaDispatcher::new(Query);
                let run = |document: &str, variables: serde_json::Value| {
                    let request = necrassrs::Request::new(document)
                        .with_variables(serde_json::from_value(variables).unwrap());
                    serde_json::to_value(futures::executor::block_on(
                        necrassrs::execute(&schema, &request, &dispatcher, &()),
                    )).unwrap()
                };
                assert_eq!(run(r#"{ min: integer(value: -2147483648) max: integer(value: 2147483647) float(value: 2) text(value: "Sheri") boolean(value: true) id(value: "001") numeric: id(value: -42) }"#, serde_json::json!({})),
                    serde_json::json!({"data": {"min": -2147483648i64, "max": 2147483647, "float": 2.0, "text": "Sheri", "boolean": true, "id": "001", "numeric": "-42"}}));
                let document = "query($i: Int!, $f: Float!, $id: ID!) { integer(value: $i) float(value: $f) id(value: $id) }";
                assert_eq!(run("{ nullable nitems required(value: [1, null]) strict(value: 2) }", serde_json::json!({})),
                    serde_json::json!({"data": {"nullable": null, "nitems": null, "required": [1, null], "strict": [2]}}));
                assert_eq!(run("{ nullable(value: [1, null]) nitems(value: 2) required(value: []) strict(value: []) }", serde_json::json!({})),
                    serde_json::json!({"data": {"nullable": [1, null], "nitems": [2], "required": [], "strict": []}}));
                assert_eq!(run("{ nullable(value: null) nitems(value: null) }", serde_json::json!({})),
                    serde_json::json!({"data": {"nullable": null, "nitems": null}}));
                for (variables, expected) in [
                    (serde_json::json!({"i": 1, "f": 1, "id": 42}), serde_json::json!({"integer": 1, "float": 1.0, "id": "42"})),
                    (serde_json::json!({"i": 1.0, "f": 1.5, "id": "00042"}), serde_json::json!({"integer": 1, "float": 1.5, "id": "00042"})),
                    (serde_json::json!({"i": -2147483648.0, "f": 1.5, "id": 42.0}), serde_json::json!({"integer": -2147483648i64, "float": 1.5, "id": "42"})),
                    (serde_json::json!({"i": 2147483647.0, "f": 1.5, "id": u64::MAX}), serde_json::json!({"integer": 2147483647, "float": 1.5, "id": u64::MAX.to_string()})),
                ] {
                    assert_eq!(run(document, variables), serde_json::json!({"data": expected}));
                }
                for document in [
                    "{ integer(value: 2147483648) }", "{ integer(value: -2147483649) }",
                    "{ integer(value: 1.0) }", "{ float(value: true) }",
                    "{ id(value: 1.5) }", "{ boolean(value: 1) }", "{ text(value: 1) }",
                    "{ required(value: null) }", "{ nitems(value: [null]) }", "{ strict(value: [null]) }",
                ] {
                    let response = run(document, serde_json::json!({}));
                    assert!(response.get("data").is_none(), "{document}: {response}");
                    assert!(!response["errors"].as_array().unwrap().is_empty());
                }
                for variables in [
                    serde_json::json!({"i": 2147483648i64, "f": 1, "id": 1}),
                    serde_json::json!({"i": 2147483648.0, "f": 1, "id": 1}),
                    serde_json::json!({"i": -2147483649.0, "f": 1, "id": 1}),
                    serde_json::json!({"i": 1.5, "f": 1, "id": 1}),
                    serde_json::json!({"i": 1, "f": "1", "id": 1}),
                    serde_json::json!({"i": 1, "f": 1, "id": true}),
                    serde_json::json!({"i": 1, "f": 1, "id": 1.5}),
                ] {
                    let response = run(document, variables);
                    assert!(response.get("data").is_none(), "{response}");
                    assert!(!response["errors"].as_array().unwrap().is_empty());
                }
            }
        "##;
        assert_consumer(&format!("mod generated {{ {generated} }}"), consumer, true);
    }

    #[test]
    fn generated_consumer_preserves_coerced_defaults_presence_and_enum_kinds() {
        let schema = Schema::parse_and_validate(
            r#"
                enum Status { OPEN CLOSED }
                input Filter {
                    status: Status = OPEN
                    optional: Int
                    matrix: [[Int]] = 3
                }
                type Query {
                    inspect(value: Int = 7, raw: Int, filter: Filter = {}): [String!]!
                    echo(status: Status!): Status!
                }
            "#,
            "schema.graphql",
        )
        .unwrap();
        let generated = super::generate(&schema).unwrap();
        let consumer = r##"
            use generated::{resolvers::QueryResolver, types};
            use necrassrs::GraphQLInput;
            use std::sync::atomic::{AtomicUsize, Ordering};

            struct Query;
            static CALLS: AtomicUsize = AtomicUsize::new(0);

            fn presence(value: GraphQLInput<i32>) -> String {
                match value {
                    GraphQLInput::Undefined => "undefined".into(),
                    GraphQLInput::Null => "null".into(),
                    GraphQLInput::Value(value) => value.to_string(),
                }
            }

            impl QueryResolver<()> for Query {
                async fn inspect(
                    &self, _: &(), args: types::Query::inspect::Args,
                ) -> Result<Vec<String>, necrassrs::ResolverError> {
                    CALLS.fetch_add(1, Ordering::Relaxed);
                    let mut result = vec![presence(args.value), presence(args.raw)];
                    match args.filter {
                        GraphQLInput::Undefined => result.push("undefined".into()),
                        GraphQLInput::Null => result.push("null".into()),
                        GraphQLInput::Value(filter) => {
                            result.push(match filter.status {
                                GraphQLInput::Value(types::Status::OPEN) => "OPEN",
                                GraphQLInput::Value(types::Status::CLOSED) => "CLOSED",
                                GraphQLInput::Null => "null",
                                GraphQLInput::Undefined => "undefined",
                            }.into());
                            result.push(presence(filter.optional));
                            result.push(match filter.matrix {
                                GraphQLInput::Value(value) => format!("{value:?}"),
                                GraphQLInput::Null => "null".into(),
                                GraphQLInput::Undefined => "undefined".into(),
                            });
                        }
                    }
                    Ok(result)
                }

                async fn echo(
                    &self, _: &(), args: types::Query::echo::Args,
                ) -> Result<types::Status, necrassrs::ResolverError> {
                    CALLS.fetch_add(1, Ordering::Relaxed);
                    Ok(args.status)
                }
            }

            fn main() {
                let schema = necrassrs::Schema::parse_and_validate(generated::SDL, "schema.graphql").unwrap();
                let dispatcher = generated::dispatch::SchemaDispatcher::new(Query);
                let run = |document: &str, variables: serde_json::Value| {
                    let request = necrassrs::Request::new(document)
                        .with_variables(serde_json::from_value(variables).unwrap());
                    serde_json::to_value(futures::executor::block_on(
                        necrassrs::execute(&schema, &request, &dispatcher, &()),
                    )).unwrap()
                };
                for (document, variables, expected) in [
                    ("{ inspect }", serde_json::json!({}),
                        serde_json::json!(["7", "undefined", "OPEN", "undefined", "[Some([Some(3)])]"])),
                    ("{ inspect(value: null, raw: null, filter: null) }", serde_json::json!({}),
                        serde_json::json!(["null", "null", "null"])),
                    ("query($v: Int, $f: Filter) { inspect(value: $v, filter: $f) }", serde_json::json!({}),
                        serde_json::json!(["7", "undefined", "OPEN", "undefined", "[Some([Some(3)])]"])),
                    ("query($v: Int = 9, $f: Filter = {optional: 5, matrix: 4}) { inspect(value: $v, filter: $f) }", serde_json::json!({}),
                        serde_json::json!(["9", "undefined", "OPEN", "5", "[Some([Some(4)])]"])),
                    ("query($f: Filter) { inspect(filter: $f) }", serde_json::json!({"f": {"status": null, "optional": null, "matrix": null}}),
                        serde_json::json!(["7", "undefined", "null", "null", "null"])),
                    ("query($f: Filter) { inspect(filter: $f) }", serde_json::json!({"f": {"status": "CLOSED", "optional": 2, "matrix": [[1, null], null]}}),
                        serde_json::json!(["7", "undefined", "CLOSED", "2", "[Some([Some(1), None]), None]"])),
                ] {
                    assert_eq!(run(document, variables), serde_json::json!({"data": {"inspect": expected}}), "{document}");
                }
                assert_eq!(run("{ echo(status: CLOSED) }", serde_json::json!({})), serde_json::json!({"data": {"echo": "CLOSED"}}));
                assert_eq!(run("query($s: Status!) { echo(status: $s) }", serde_json::json!({"s": "OPEN"})), serde_json::json!({"data": {"echo": "OPEN"}}));
                for (document, variables) in [
                    (r#"{ echo(status: "OPEN") }"#, serde_json::json!({})),
                    ("{ echo(status: UNKNOWN) }", serde_json::json!({})),
                    ("query($s: Status!) { echo(status: $s) }", serde_json::json!({"s": "UNKNOWN"})),
                    ("query($s: Status!) { echo(status: $s) }", serde_json::json!({"s": 1})),
                    ("{ inspect(filter: {unknown: 1}) }", serde_json::json!({})),
                    ("query($f: Filter) { inspect(filter: $f) }", serde_json::json!({"f": {"unknown": 1}})),
                ] {
                    let before = CALLS.load(Ordering::Relaxed);
                    let response = run(document, variables);
                    assert!(response.get("data").is_none(), "{document}: {response}");
                    assert!(!response["errors"].as_array().unwrap().is_empty());
                    assert_eq!(CALLS.load(Ordering::Relaxed), before);
                }
            }
        "##;
        assert_consumer(&format!("mod generated {{ {generated} }}"), consumer, true);
    }

    #[test]
    fn embedded_sdl_preserves_definitions_and_extensions_from_multiple_sources() {
        let schema = Schema::builder()
            .parse(
                "schema { query: ReadRoot } type ReadRoot { hello: String! }",
                "root.graphql",
            )
            .parse(
                "extend type ReadRoot { greet(name: String!): String! }",
                "greeting.graphql",
            )
            .build()
            .expect("the test schema must build")
            .validate()
            .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("generation must succeed");
        let consumer = r#"
            fn main() {
                let schema = necrassrs::Schema::parse_and_validate(
                    generated::SDL, "embedded.graphql",
                ).unwrap();
                assert_eq!(schema.schema_definition.query.as_ref().unwrap().as_str(), "ReadRoot");
                let root = schema.get_object("ReadRoot").unwrap();
                assert_eq!(root.fields["hello"].ty.to_string(), "String!");
                let greet = &root.fields["greet"];
                assert_eq!(greet.ty.to_string(), "String!");
                assert_eq!(greet.arguments.len(), 1);
                assert_eq!(greet.arguments[0].name.as_str(), "name");
                assert_eq!(greet.arguments[0].ty.to_string(), "String!");
            }
        "#;
        assert_consumer(
            &format!("pub mod generated {{ {generated} }}"),
            consumer,
            true,
        );
    }

    #[test]
    fn generated_dispatch_uses_schema_coordinates_and_preserves_errors() {
        let sdl = "schema { query: ReadRoot } type ReadRoot { greet(who: String!): String! fail: String! pending: String! }";
        let schema = Schema::parse_and_validate(sdl, "schema.graphql")
            .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("generation must succeed");
        let consumer = r#"
            use generated::{resolvers::ReadRootResolver, types};
            use necrassrs::Dispatcher;

            struct Query;
            impl ReadRootResolver<()> for Query {
                async fn greet<'a>(
                    &'a self, _: &'a (), args: types::ReadRoot::greet::Args,
                ) -> Result<String, necrassrs::ResolverError> {
                    Ok(format!("Hello, {}", args.who))
                }

                async fn fail<'a>(
                    &'a self, _: &'a (), _: types::ReadRoot::fail::Args,
                ) -> Result<String, necrassrs::ResolverError> {
                    Err(necrassrs::ResolverError::new("Greeting failed.")
                        .with_extension("code", "GREETING_FAILED"))
                }
            }

            fn main() {
                let schema = necrassrs::Schema::parse_and_validate(generated::SDL, "schema.graphql").unwrap();
                let dispatcher = generated::dispatch::SchemaDispatcher::new(Query);
                let run = |document| {
                    let request = necrassrs::Request::new(document);
                    serde_json::to_value(futures::executor::block_on(
                        necrassrs::execute(&schema, &request, &dispatcher, &())
                    )).unwrap()
                };
                let failed = run("{ problem: fail }");
                assert_eq!(failed["data"], serde_json::Value::Null);
                assert_eq!(failed["errors"][0]["message"], "Greeting failed.");
                assert_eq!(failed["errors"][0]["extensions"]["code"], "GREETING_FAILED");
                assert_eq!(failed["errors"][0]["path"], serde_json::json!(["problem"]));
                assert_eq!(failed["errors"][0]["locations"], serde_json::json!([{ "line": 1, "column": 12 }]));
                assert_eq!(run("{ greeting: greet(who: \"Sheri\") }"),
                    serde_json::json!({ "data": { "greeting": "Hello, Sheri" } }));

                for (parent_type, field, arguments) in [
                    ("Query", "greet", necrassrs::JsonMap::new()),
                    ("ReadRoot", "missing", necrassrs::JsonMap::new()),
                    ("ReadRoot", "greet", necrassrs::JsonMap::new()),
                    ("ReadRoot", "greet", [("who".into(), necrassrs::JsonValue::Null)].into_iter().collect()),
                    ("ReadRoot", "greet", [("who".into(), necrassrs::JsonValue::from(42))].into_iter().collect()),
                ] {
                    assert!(futures::executor::block_on(dispatcher.resolve(
                        &(), necrassrs::FieldCoordinate { parent_type, field }, &arguments,
                    )).is_err());
                }
            }
        "#;
        let source = format!("mod generated {{ {generated} }}");
        assert_consumer(&source, consumer, true);
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn selected_unimplemented_field_aborts_dev_and_release_consumers() {
        use std::{fs, os::unix::process::ExitStatusExt, path::Path, process::Command};

        let schema = Schema::parse_and_validate(
            "type Query { hello: String! pending: String! }",
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("generation must succeed");
        let consumer = r#"
            struct Query;
            impl generated::resolvers::QueryResolver<()> for Query {
                async fn hello<'a>(
                    &'a self, _: &'a (), _: generated::types::Query::hello::Args,
                ) -> Result<String, necrassrs::ResolverError> {
                    Ok(String::from("Hello, Sheri"))
                }
            }

            fn main() {
                let schema = necrassrs::Schema::parse_and_validate(
                    generated::SDL, "schema.graphql",
                ).unwrap();
                let dispatcher = generated::dispatch::SchemaDispatcher::new(Query);
                let document = std::env::args().nth(1).unwrap();
                let request = necrassrs::Request::new(document);
                let response = futures::executor::block_on(
                    necrassrs::execute(&schema, &request, &dispatcher, &()),
                );
                println!("{}", serde_json::to_string(&response).unwrap());
            }
        "#;
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let directory = std::env::temp_dir().join(format!(
            "necrassrs-abort-consumer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir(&directory).unwrap();
        let apollo = workspace
            .join("vendor/apollo-compiler")
            .canonicalize()
            .unwrap();
        let runtime = workspace.join("crates/necrassrs").canonicalize().unwrap();
        fs::write(
            directory.join("Cargo.toml"),
            format!(
                r#"
                    [package]
                    name = "necrassrs-abort-consumer"
                    version = "0.0.0"
                    edition = "2024"
                    [workspace]
                    [[bin]]
                    name = "necrassrs-abort-consumer"
                    path = "main.rs"
                    [dependencies]
                    necrassrs = {{ path = {runtime:?} }}
                    futures = "0.3"
                    serde_json = "1.0"
                    [patch.crates-io]
                    apollo-compiler = {{ path = {apollo:?} }}
                    [profile.dev]
                    panic = "abort"
                    [profile.release]
                    panic = "abort"
                    [lints.rust]
                    warnings = "deny"
                "#,
            ),
        )
        .unwrap();
        fs::copy(workspace.join("Cargo.lock"), directory.join("Cargo.lock")).unwrap();
        fs::write(
            directory.join("main.rs"),
            format!("mod generated {{ {generated} }}\n{consumer}"),
        )
        .unwrap();
        let target = workspace.join("target/abort-consumer");
        for (profile, output_directory) in [("dev", "debug"), ("release", "release")] {
            let build = Command::new(env!("CARGO"))
                .current_dir(&directory)
                .args(["build", "--offline", "--profile", profile, "--target-dir"])
                .arg(&target)
                .output()
                .expect("Cargo must be available");
            assert!(
                build.status.success(),
                "{}",
                String::from_utf8_lossy(&build.stderr)
            );
            let executable = target
                .join(output_directory)
                .join("necrassrs-abort-consumer");
            let hello = Command::new(&executable)
                .current_dir(&directory)
                .arg("{ hello }")
                .output()
                .unwrap();
            let pending = Command::new(&executable)
                .current_dir(&directory)
                .arg("{ pending }")
                .output()
                .unwrap();
            assert!(
                hello.status.success(),
                "{profile}: {}",
                String::from_utf8_lossy(&hello.stderr)
            );
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&hello.stdout).unwrap(),
                serde_json::json!({ "data": { "hello": "Hello, Sheri" } }),
            );
            // SIGABRT is 6 on Linux and macOS; a normal panic exit is insufficient.
            assert_eq!(pending.status.signal(), Some(6), "{profile}: {pending:?}");
            assert!(
                pending.stdout.is_empty(),
                "{profile}: execution must not return a response"
            );
            assert!(
                String::from_utf8_lossy(&pending.stderr)
                    .contains("Resolver Query.pending is not implemented"),
                "{profile}: {}",
                String::from_utf8_lossy(&pending.stderr),
            );
        }
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn resolver_return_types_follow_graphql_wrappers() {
        let schema = Schema::parse_and_validate(
            r#"
                enum Status { OPEN CLOSED }
                type Query {
                    intResult: Int!
                    floatResult: Float!
                    stringResult: String!
                    booleanResult: Boolean!
                    idResult: ID!
                    statusResult: Status!
                    nullableStatusResult: Status
                    nullableListResult: [Int]
                    requiredListResult: [Int]!
                    nullableNonNullItemsResult: [Int!]
                    requiredNonNullItemsResult: [Int!]!
                }
            "#,
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let query = schema.get_object("Query").unwrap();
        let cases = [
            ("intResult", quote! { ::core::primitive::i32 }),
            ("floatResult", quote! { ::core::primitive::f64 }),
            ("stringResult", quote! { ::std::string::String }),
            ("booleanResult", quote! { ::core::primitive::bool }),
            ("idResult", quote! { ::necrassrs::Id }),
            ("statusResult", quote! { super::types::r#Status }),
            (
                "nullableStatusResult",
                quote! { ::core::option::Option<super::types::r#Status> },
            ),
            (
                "nullableListResult",
                quote! { ::core::option::Option<::std::vec::Vec<::core::option::Option<::core::primitive::i32>>> },
            ),
            (
                "requiredListResult",
                quote! { ::std::vec::Vec<::core::option::Option<::core::primitive::i32>> },
            ),
            (
                "nullableNonNullItemsResult",
                quote! { ::core::option::Option<::std::vec::Vec<::core::primitive::i32>> },
            ),
            (
                "requiredNonNullItemsResult",
                quote! { ::std::vec::Vec<::core::primitive::i32> },
            ),
        ];

        for (field_name, expected) in cases {
            let field = &query.fields[field_name];
            let actual = super::resolver_return_type(
                &schema,
                query.name.as_str(),
                field_name,
                &field.ty,
                &quote! { super::types },
            )
            .unwrap_or_else(|error| panic!("{field_name}: {error}"));
            assert_eq!(
                normalize_type(actual),
                normalize_type(expected),
                "{field_name}"
            );
        }
    }

    #[test]
    fn generated_types_include_enum_input_and_argument_contracts() {
        let schema = Schema::parse_and_validate(
            r#"
                enum Status { OPEN CLOSED }
                input Nested { name: String! }
                input Filter {
                    required: Boolean!
                    optionalId: ID
                    statuses: [Status]
                    nested: Nested!
                }
                type Query {
                    inspect(
                        status: Status!
                        optionalString: String
                        nullableList: [Int]
                        requiredList: [Int]!
                        filter: Filter
                    ): String!
                }
            "#,
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let generated = super::generate(&schema)
            .expect("supported input contracts must be generated")
            .to_string();

        for expected in [
            "pub enum r#Status",
            "r#OPEN",
            "r#CLOSED",
            "pub struct r#Nested",
            "pub struct r#Filter",
            "fn _from_graphql_value",
            "as_object",
            "object . get",
            "pub r#required : :: core :: primitive :: bool",
            "pub r#optionalId : :: necrassrs :: GraphQLInput < :: necrassrs :: Id >",
            "pub r#statuses : :: necrassrs :: GraphQLInput",
            "self :: r#Nested :: _from_graphql_value",
            "pub r#status : super :: super :: r#Status",
            "pub r#optionalString : :: necrassrs :: GraphQLInput",
            "pub r#nullableList : :: necrassrs :: GraphQLInput",
            "pub r#requiredList : :: std :: vec :: Vec",
            "pub r#filter : :: necrassrs :: GraphQLInput < super :: super :: r#Filter >",
        ] {
            assert!(
                generated.contains(expected),
                "missing `{expected}` in {generated}"
            );
        }
    }

    #[test]
    fn generated_one_of_types_use_non_null_variant_payloads() {
        let schema = Schema::parse_and_validate(
            r#"
                directive @oneOf on INPUT_OBJECT
                enum Status { OPEN CLOSED }
                input Nested { name: String! }
                input Choice @oneOf {
                    number: Int
                    decimal: Float
                    text: String
                    flag: Boolean
                    id: ID
                    status: Status
                    nested: Nested
                    items: [Int]
                    requiredItems: [Int!]
                    matrix: [[Int!]]
                }
                input Filter { choice: Choice required: Choice! }
                type Query { inspect(choice: Choice, required: Choice!): String! }
            "#,
            "schema.graphql",
        )
        .expect("the OneOf test schema must be valid");
        let generated = super::generate_types(&schema)
            .expect("OneOf type generation must succeed")
            .to_token_stream();
        let file = syn::parse2::<syn::File>(generated.clone())
            .expect("generated types must be valid Rust syntax");
        let types = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Mod(module) if module.ident == "types" => {
                    module.content.as_ref().map(|(_, items)| items)
                }
                _ => None,
            })
            .expect("generated types module must exist");
        let choice = types
            .iter()
            .find_map(|item| match item {
                syn::Item::Enum(item) if item.ident == "r#Choice" => Some(item),
                _ => None,
            })
            .expect("OneOf Choice must be generated as an enum, not a struct");
        let expected = syn::parse2::<syn::ItemEnum>(quote! {
            #[allow(non_camel_case_types)]
            pub enum r#Choice {
                r#number(::core::primitive::i32),
                r#decimal(::core::primitive::f64),
                r#text(::std::string::String),
                r#flag(::core::primitive::bool),
                r#id(::necrassrs::Id),
                r#status(self::r#Status),
                r#nested(self::r#Nested),
                r#items(::std::vec::Vec<::core::option::Option<::core::primitive::i32>>),
                r#requiredItems(::std::vec::Vec<::core::primitive::i32>),
                r#matrix(::std::vec::Vec<::core::option::Option<::std::vec::Vec<::core::primitive::i32>>>),
            }
        })
        .unwrap();
        assert_eq!(
            choice.to_token_stream().to_string(),
            expected.to_token_stream().to_string(),
            "selected payloads must be non-null while retaining nested list nullability"
        );

        assert_consumer_compiles(
            &generated.to_string(),
            r#"
                pub fn main() {
                    let _ = types::Filter::_from_graphql_value(&necrassrs::JsonValue::Null);
                    let _: types::Choice = types::Choice::number(1);
                    let _: types::Choice = types::Choice::items(vec![Some(1), None]);
                    let _: types::Choice = types::Choice::requiredItems(vec![1]);
                    let _: types::Choice = types::Choice::matrix(vec![None, Some(vec![1])]);
                    let _ = types::Filter {
                        choice: necrassrs::GraphQLInput::Null,
                        required: types::Choice::flag(true),
                    };
                    for choice in [
                        necrassrs::GraphQLInput::Undefined,
                        necrassrs::GraphQLInput::Null,
                        necrassrs::GraphQLInput::Value(types::Choice::text(String::new())),
                    ] {
                        let _ = types::Query::inspect::Args {
                            choice,
                            required: types::Choice::id(necrassrs::Id::from("001")),
                        };
                    }
                }
            "#,
        );
    }

    #[test]
    fn generated_one_of_conversion_requires_one_non_null_known_field() {
        let schema = Schema::parse_and_validate(
            r#"
                directive @oneOf on INPUT_OBJECT
                enum Status { OPEN CLOSED }
                input Nested { name: String! }
                input Choice @oneOf {
                    number: Int
                    status: Status
                    nested: Nested
                    items: [Int]
                    requiredItems: [Int!]
                }
                type Query { inspect(choice: Choice): String! }
            "#,
            "schema.graphql",
        )
        .expect("the OneOf test schema must be valid");
        let generated = super::generate_types(&schema)
            .expect("OneOf type generation must succeed")
            .to_token_stream()
            .to_string();
        assert_consumer(
            &generated,
            r#"
                macro_rules! json {
                    ($($tokens:tt)*) => {
                        serde_json::from_value::<necrassrs::JsonValue>(
                            serde_json::json!($($tokens)*),
                        ).unwrap()
                    };
                }

                fn main() {
                    let value = types::Choice::_from_graphql_value(&json!({"number": 7}));
                    assert!(matches!(value, Ok(types::Choice::number(7))));
                    let value = types::Choice::_from_graphql_value(&json!({"status": "OPEN"}));
                    assert!(matches!(value, Ok(types::Choice::status(types::Status::OPEN))));
                    let value = types::Choice::_from_graphql_value(&json!({"nested": {"name": "Sheri"}}));
                    let Ok(types::Choice::nested(nested)) = value else {
                        panic!("nested input must select its variant");
                    };
                    assert_eq!(nested.name, "Sheri");
                    let value = types::Choice::_from_graphql_value(&json!({"items": [1, null, 2]}));
                    let Ok(types::Choice::items(items)) = value else {
                        panic!("nullable list items must be preserved");
                    };
                    assert_eq!(items, vec![Some(1), None, Some(2)]);

                    for invalid in [
                        json!(null), json!([]), json!({}),
                        json!({"number": null}), json!({"unknown": 1}),
                        json!({"number": 1, "status": "OPEN"}),
                        json!({"number": 1, "status": null}),
                        json!({"number": "wrong"}), json!({"status": "UNKNOWN"}),
                        json!({"nested": {}}), json!({"requiredItems": [null]}),
                    ] {
                        assert!(
                            types::Choice::_from_graphql_value(&invalid).is_err(),
                            "invalid OneOf input was accepted: {invalid}",
                        );
                    }
                }
            "#,
            true,
        );
    }

    #[test]
    fn generated_recursive_inputs_box_fields_and_convert_finite_values() {
        let schema = Schema::parse_and_validate(
            r#"
                input Node {
                    next: Node
                    choice: Choice
                    children: [Node!]
                }
                input Choice @oneOf { number: Int node: Node }
                type Query { inspect(input: Node): String! }
            "#,
            "schema.graphql",
        )
        .expect("the recursive input schema must be valid");
        let generated = super::generate_types(&schema)
            .expect("recursive input type generation must succeed")
            .to_token_stream()
            .to_string();
        assert_consumer(
            &generated,
            r##"
                use necrassrs::GraphQLInput;
                use types::{Choice, Node};

                fn parse_node(source: &str) -> Node {
                    let value = serde_json::from_str::<necrassrs::JsonValue>(source).unwrap();
                    Node::_from_graphql_value(&value)
                        .unwrap_or_else(|_| panic!("finite input must convert: {source}"))
                }

                fn main() {
                    let node = parse_node("{}");
                    let next: GraphQLInput<Box<Node>> = node.next;
                    let choice: GraphQLInput<Box<Choice>> = node.choice;
                    let children: GraphQLInput<Vec<Node>> = node.children;
                    assert!(matches!(next, GraphQLInput::Undefined));
                    assert!(matches!(choice, GraphQLInput::Undefined));
                    assert!(matches!(children, GraphQLInput::Undefined));

                    let node = parse_node(r#"{"next":null,"choice":null,"children":null}"#);
                    assert!(matches!(node.next, GraphQLInput::Null));
                    assert!(matches!(node.choice, GraphQLInput::Null));
                    assert!(matches!(node.children, GraphQLInput::Null));

                    let node = parse_node(r#"{
                        "next": {},
                        "choice": {"node": {"choice": {"number": 7}}},
                        "children": [{}]
                    }"#);
                    let GraphQLInput::Value(next) = node.next else {
                        panic!("supplied next must retain its presence");
                    };
                    let next: Box<Node> = next;
                    assert!(matches!(next.next, GraphQLInput::Undefined));
                    let GraphQLInput::Value(choice) = node.choice else {
                        panic!("supplied choice must retain its presence");
                    };
                    let Choice::node(inner) = *choice else {
                        panic!("OneOf must select the recursive node variant");
                    };
                    let inner: Box<Node> = inner;
                    let GraphQLInput::Value(choice) = inner.choice else {
                        panic!("nested choice must convert");
                    };
                    assert!(matches!(*choice, Choice::number(7)));
                    let GraphQLInput::Value(children) = node.children else {
                        panic!("supplied children must convert without boxing list items");
                    };
                    let children: Vec<Node> = children;
                    assert_eq!(children.len(), 1);
                    assert!(matches!(children[0].next, GraphQLInput::Undefined));

                    let args = types::Query::inspect::Args {
                        input: GraphQLInput::Value(parse_node("{}")),
                    };
                    let _: GraphQLInput<Node> = args.input;
                }
            "##,
            true,
        );
    }

    #[test]
    fn generated_dispatch_serializes_id_results_as_strings() {
        let schema = Schema::parse_and_validate("type Query { id: ID! }", "schema.graphql")
            .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("generation must succeed");
        let consumer = r#"
            use necrassrs::Dispatcher;

            pub struct Query(&'static str);
            impl resolvers::QueryResolver<()> for Query {
                async fn id<'a>(
                    &'a self, _: &'a (), _: types::Query::id::Args,
                ) -> Result<necrassrs::Id, necrassrs::ResolverError> {
                    Ok(necrassrs::Id::from(self.0))
                }
            }

            fn main() {
                for expected in ["001", "", "Sheri"] {
                    let dispatcher = dispatch::SchemaDispatcher::new(Query(expected));
                    let actual = futures::executor::block_on(dispatcher.resolve(
                        &(),
                        necrassrs::FieldCoordinate { parent_type: "Query", field: "id" },
                        &necrassrs::JsonMap::new(),
                    )).unwrap_or_else(|_| panic!("ID result conversion must succeed"));
                    let necrassrs::ResolvedValue::Json(actual) = actual else {
                        panic!("ID result must be a JSON leaf");
                    };
                    assert_eq!(actual, necrassrs::JsonValue::from(expected));
                }
            }
        "#;

        assert_consumer(&generated, consumer, true);
    }

    #[test]
    fn generated_dispatch_serializes_enum_results_as_graphql_names() {
        let schema = Schema::parse_and_validate(
            "enum Status { OPEN CLOSED } type Query { status: Status! }",
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("generation must succeed");
        let consumer = r#"
            use necrassrs::Dispatcher;

            pub struct Query(bool);
            impl resolvers::QueryResolver<()> for Query {
                async fn status<'a>(
                    &'a self, _: &'a (), _: types::Query::status::Args,
                ) -> Result<types::Status, necrassrs::ResolverError> {
                    Ok(if self.0 { types::Status::OPEN } else { types::Status::CLOSED })
                }
            }

            fn main() {
                for (open, expected) in [(true, "OPEN"), (false, "CLOSED")] {
                    let dispatcher = dispatch::SchemaDispatcher::new(Query(open));
                    let actual = futures::executor::block_on(dispatcher.resolve(
                        &(),
                        necrassrs::FieldCoordinate { parent_type: "Query", field: "status" },
                        &necrassrs::JsonMap::new(),
                    )).unwrap_or_else(|_| panic!("enum result conversion must succeed"));
                    let necrassrs::ResolvedValue::Json(actual) = actual else {
                        panic!("enum result must be a JSON leaf");
                    };
                    assert_eq!(actual, necrassrs::JsonValue::from(expected));
                }
            }
        "#;

        assert_consumer(&generated, consumer, true);
    }

    #[test]
    fn generated_float_list_error_preserves_nullable_items_and_index_path() {
        let schema = Schema::parse_and_validate("type Query { values: [Float] }", "schema.graphql")
            .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("generation must succeed");
        let consumer = r#"
            use generated::{resolvers::QueryResolver, types};

            struct Query(f64);
            impl QueryResolver<()> for Query {
                async fn values<'a>(
                    &'a self, _: &'a (), _: types::Query::values::Args,
                ) -> Result<Option<Vec<Option<f64>>>, necrassrs::ResolverError> {
                    Ok(Some(vec![Some(1.5), Some(self.0), Some(2.5)]))
                }
            }

            fn main() {
                let schema = necrassrs::Schema::parse_and_validate(
                    generated::SDL, "schema.graphql",
                ).unwrap();
                let request = necrassrs::Request::new("{ numbers: values }");

                for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                    let dispatcher = generated::dispatch::SchemaDispatcher::new(Query(value));
                    let response = futures::executor::block_on(
                        necrassrs::execute(&schema, &request, &dispatcher, &()),
                    );
                    let response = serde_json::to_value(response).unwrap();

                    assert_eq!(
                        response["data"],
                        serde_json::json!({ "numbers": [1.5, null, 2.5] }),
                        "non-finite Float {value}: {response}",
                    );
                    let errors = response["errors"].as_array().expect("an item error is required");
                    assert_eq!(errors.len(), 1);
                    assert_eq!(errors[0]["path"], serde_json::json!(["numbers", 1]));
                    assert!(errors[0]["message"].as_str().is_some_and(|message| !message.is_empty()));
                }
            }
        "#;

        assert_consumer(&format!("mod generated {{ {generated} }}"), consumer, true);
    }

    const FLOAT_RESULT_CONSUMER: &str = r#"
        use necrassrs::Dispatcher;

        pub struct Query(f64);
        impl resolvers::QueryResolver<()> for Query {
            async fn value<'a>(
                &'a self, _: &'a (), _: types::Query::value::Args,
            ) -> Result<f64, necrassrs::ResolverError> {
                Ok(self.0)
            }
        }

        fn resolve(value: f64) -> Result<necrassrs::ResolvedValue, necrassrs::ResolverError> {
            let dispatcher = dispatch::SchemaDispatcher::new(Query(value));
            futures::executor::block_on(dispatcher.resolve(
                &(),
                necrassrs::FieldCoordinate { parent_type: "Query", field: "value" },
                &necrassrs::JsonMap::new(),
            ))
        }
    "#;

    #[test]
    fn generated_dispatch_serializes_finite_float_results() {
        let schema = Schema::parse_and_validate("type Query { value: Float! }", "schema.graphql")
            .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("Float results must be supported");
        let main = r#"
            fn main() {
                for value in [0.0, -0.0, 1.5, -2.5, f64::MIN, f64::MAX] {
                    let actual = resolve(value)
                        .unwrap_or_else(|_| panic!("finite Float {value} must succeed"));
                    let necrassrs::ResolvedValue::Json(actual) = actual else {
                        panic!("finite Float must be a JSON leaf");
                    };
                    assert!(actual.is_number(), "finite Float {value} must be a JSON number");
                    assert_eq!(actual, necrassrs::JsonValue::from(value));
                }
            }
        "#;

        assert_consumer(&generated, &format!("{FLOAT_RESULT_CONSUMER}{main}"), true);
    }

    #[test]
    fn generated_dispatch_rejects_non_finite_float_results() {
        let schema = Schema::parse_and_validate("type Query { value: Float! }", "schema.graphql")
            .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("Float results must be supported");
        let main = r#"
            fn main() {
                for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                    assert!(
                        matches!(resolve(value), Ok(necrassrs::ResolvedValue::Error(_))),
                        "non-finite Float {value} must retain a conversion error",
                    );
                }
            }
        "#;

        assert_consumer(&generated, &format!("{FLOAT_RESULT_CONSUMER}{main}"), true);
    }

    fn assert_generated_result_cases(
        definition: &str,
        result_type: &str,
        rust_type: &str,
        cases: &[(&str, Option<&str>)],
    ) {
        let schema = Schema::parse_and_validate(
            format!("{definition} type Query {{ value: {result_type} }}"),
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("generation must succeed");
        let arms = cases
            .iter()
            .enumerate()
            .map(|(index, (value, _))| format!("{index} => {value},"))
            .collect::<String>();
        let assertions = cases
            .iter()
            .enumerate()
            .map(|(index, (_, expected))| {
                let assertion = match expected {
                    Some(expected) => format!(
                        "assert!(actual.get(\"errors\").is_none(), \"case {index}: {{actual}}\");
                         assert_eq!(actual[\"data\"][\"value\"], serde_json::json!({expected}));"
                    ),
                    None => format!(
                        "assert_eq!(actual[\"errors\"].as_array().unwrap().len(), 1, \"case {index}\");"
                    ),
                };
                format!(
                    "let dispatcher = dispatch::SchemaDispatcher::new(Query({index}));
                     let actual = futures::executor::block_on(necrassrs::execute(
                         &schema, &request, &dispatcher, &(),
                     ));
                     let actual = serde_json::to_value(actual).unwrap();
                     {assertion}"
                )
            })
            .collect::<String>();
        let consumer = format!(
            "pub struct Query(usize);
             impl resolvers::QueryResolver<()> for Query {{
                 async fn value<'a>(
                     &'a self, _: &'a (), _: types::Query::value::Args,
                 ) -> Result<{rust_type}, necrassrs::ResolverError> {{
                     Ok(match self.0 {{ {arms} _ => unreachable!() }})
                 }}
             }}
             fn main() {{
                 let schema = necrassrs::Schema::parse_and_validate(SDL, \"schema.graphql\").unwrap();
                 let request = necrassrs::Request::new(\"{{ value }}\");
                 {assertions}
             }}"
        );

        assert_consumer(&generated, &consumer, true);
    }

    #[test]
    fn generated_result_wrappers_preserve_nullable_id_values() {
        assert_generated_result_cases(
            "",
            "ID",
            "Option<necrassrs::Id>",
            &[
                ("None", Some("null")),
                ("Some(necrassrs::Id::from(\"001\"))", Some("\"001\"")),
            ],
        );
    }

    #[test]
    fn generated_result_wrappers_preserve_nullable_enum_values() {
        assert_generated_result_cases(
            "enum Status { OPEN CLOSED }",
            "Status",
            "Option<types::Status>",
            &[
                ("None", Some("null")),
                ("Some(types::Status::OPEN)", Some("\"OPEN\"")),
                ("Some(types::Status::CLOSED)", Some("\"CLOSED\"")),
            ],
        );
    }

    #[test]
    fn generated_result_wrappers_preserve_id_list_nullability() {
        for (result_type, rust_type, cases) in [
            (
                "[ID]",
                "Option<Vec<Option<necrassrs::Id>>>",
                vec![
                    ("None", Some("null")),
                    ("Some(vec![])", Some("[]")),
                    (
                        "Some(vec![Some(necrassrs::Id::from(\"001\")), None])",
                        Some("[\"001\", null]"),
                    ),
                ],
            ),
            (
                "[ID]!",
                "Vec<Option<necrassrs::Id>>",
                vec![
                    ("vec![]", Some("[]")),
                    (
                        "vec![Some(necrassrs::Id::from(\"001\")), None]",
                        Some("[\"001\", null]"),
                    ),
                ],
            ),
            (
                "[ID!]",
                "Option<Vec<necrassrs::Id>>",
                vec![
                    ("None", Some("null")),
                    ("Some(vec![])", Some("[]")),
                    (
                        "Some(vec![necrassrs::Id::from(\"001\")])",
                        Some("[\"001\"]"),
                    ),
                ],
            ),
            (
                "[ID!]!",
                "Vec<necrassrs::Id>",
                vec![
                    ("vec![]", Some("[]")),
                    ("vec![necrassrs::Id::from(\"001\")]", Some("[\"001\"]")),
                ],
            ),
        ] {
            assert_generated_result_cases("", result_type, rust_type, &cases);
        }
    }

    #[test]
    fn generated_result_wrappers_convert_nested_enum_lists() {
        assert_generated_result_cases(
            "enum Status { OPEN CLOSED }",
            "[[Status]]",
            "Option<Vec<Option<Vec<Option<types::Status>>>>>",
            &[
                ("None", Some("null")),
                ("Some(vec![])", Some("[]")),
                (
                    "Some(vec![None, Some(vec![]), Some(vec![Some(types::Status::OPEN), None, Some(types::Status::CLOSED)])])",
                    Some("[null, [], [\"OPEN\", null, \"CLOSED\"]]"),
                ),
            ],
        );
    }

    #[test]
    fn generated_result_wrappers_validate_nullable_float_values() {
        assert_generated_result_cases(
            "",
            "Float",
            "Option<f64>",
            &[
                ("None", Some("null")),
                ("Some(1.5)", Some("1.5")),
                ("Some(f64::NAN)", None),
                ("Some(f64::INFINITY)", None),
                ("Some(f64::NEG_INFINITY)", None),
            ],
        );
    }

    #[test]
    fn generated_result_wrappers_validate_float_list_items() {
        assert_generated_result_cases(
            "",
            "[Float!]!",
            "Vec<f64>",
            &[
                ("vec![]", Some("[]")),
                ("vec![1.5, -2.5]", Some("[1.5, -2.5]")),
                ("vec![1.5, f64::NAN]", None),
                ("vec![1.5, f64::INFINITY]", None),
                ("vec![1.5, f64::NEG_INFINITY]", None),
            ],
        );
        assert_generated_result_cases(
            "",
            "[[Float]]",
            "Option<Vec<Option<Vec<Option<f64>>>>>",
            &[
                (
                    "Some(vec![None, Some(vec![Some(1.5), None])])",
                    Some("[null, [1.5, null]]"),
                ),
                ("Some(vec![Some(vec![Some(1.5), Some(f64::NAN)])])", None),
            ],
        );
    }

    #[test]
    fn generated_dispatch_converts_non_null_leaf_arguments() {
        let cases: &[(&str, &str, &[&str])] = &[
            ("", "Int!", &["as_i64", "i32"]),
            ("", "Float!", &["as_f64"]),
            ("", "String!", &["as_str"]),
            ("", "Boolean!", &["as_bool"]),
            ("", "ID!", &["necrassrs :: Id", "as_i64"]),
            (
                "enum Status { OPEN CLOSED }",
                "Status!",
                &["r#OPEN", "r#CLOSED"],
            ),
        ];

        for (definition, argument_type, expected) in cases {
            let schema = Schema::parse_and_validate(
                format!("{definition}\ntype Query {{ inspect(value: {argument_type}): String! }}"),
                "schema.graphql",
            )
            .unwrap_or_else(|error| panic!("{argument_type}: {error}"));
            let generated = super::generate_dispatch(&schema)
                .unwrap_or_else(|error| panic!("{argument_type}: {error}"))
                .to_token_stream()
                .to_string();

            for token in *expected {
                assert!(
                    generated.contains(token),
                    "{argument_type}: missing `{token}` in {generated}"
                );
            }
        }
    }

    #[test]
    fn generated_dispatch_preserves_nullable_argument_presence() {
        let schema = Schema::parse_and_validate(
            "type Query { inspect(value: Int): String! }",
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let generated = super::generate_dispatch(&schema)
            .expect("nullable arguments must be supported")
            .to_token_stream()
            .to_string();

        for expected in [
            "GraphQLInput :: Undefined",
            "GraphQLInput :: Null",
            "GraphQLInput :: Value",
            "as_i64",
        ] {
            assert!(
                generated.contains(expected),
                "missing `{expected}` in {generated}"
            );
        }
    }

    #[test]
    fn generated_dispatch_converts_list_container_and_item_nullability() {
        let cases = [
            ("[Int]", true, true, "as_i64", 1),
            ("[Int]!", false, true, "as_i64", 1),
            ("[Int!]", true, false, "as_i64", 1),
            ("[Int!]!", false, false, "as_i64", 1),
            ("[[Boolean!]!]!", false, false, "as_bool", 2),
        ];

        for (argument_type, nullable_container, nullable_item, leaf, list_depth) in cases {
            let schema = Schema::parse_and_validate(
                format!("type Query {{ inspect(value: {argument_type}): String! }}"),
                "schema.graphql",
            )
            .unwrap_or_else(|error| panic!("{argument_type}: {error}"));
            let generated = super::generate_dispatch(&schema)
                .unwrap_or_else(|error| panic!("{argument_type}: {error}"))
                .to_token_stream()
                .to_string();

            assert!(
                generated.matches("as_array").count() >= list_depth,
                "{argument_type}: missing recursive list conversion in {generated}"
            );
            assert!(
                generated.contains(leaf),
                "{argument_type}: missing `{leaf}` in {generated}"
            );
            assert_eq!(
                generated.contains("GraphQLInput :: Undefined"),
                nullable_container,
                "{argument_type}: incorrect container presence conversion in {generated}"
            );
            if nullable_item {
                assert!(
                    generated.contains("Some") && generated.contains("None"),
                    "{argument_type}: nullable items must produce Option values in {generated}"
                );
            }
        }
    }

    #[test]
    fn generated_paths_preserve_case_boundaries_and_escape_rust_names() {
        let schema = Schema::parse_and_validate(
            r#"
                type Query {
                    hello(name: String!): String!
                    Hello(name: String!): String!
                    type(self: String!, _self: String!, _: String!, gen: String!): String!
                    self(name: String!): String!
                    _self(name: String!): String!
                    Args(name: String!): String!
                }
                type A_B { c(name: String!): String! }
                type A { B_c(name: String!): String! }
                type self { super(crate: String!, Self: String!): String! }
                type _self { super(crate: String!, Self: String!): String! }
                type User { hello(name: String!): String! }
                type user { hello(name: String!): String! }
                type UserResolver { hello(name: String!): String! }
            "#,
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("generation must succeed");
        let consumer = r#"
            pub struct Query;
            pub struct Context;

            impl resolvers::QueryResolver<Context> for Query {}
            impl resolvers::UserResolver<Context> for Query {}
            impl resolvers::userResolver<Context> for Query {}
            impl resolvers::UserResolverResolver<Context> for Query {}
            impl resolvers::_selfResolver<Context> for Query {}
            impl resolvers::__selfResolver<Context> for Query {}

            pub fn check() {
                let _: String = types::Query::hello::Args { name: String::new() }.name;
                let _: String = types::Query::Hello::Args { name: String::new() }.name;
                let _ = types::Query::r#type::Args {
                    _self: String::new(), __self: String::new(),
                    __: String::new(), r#gen: String::new(),
                };
                let _ = types::Query::_self::Args { name: String::new() };
                let _ = types::Query::__self::Args { name: String::new() };
                let _ = types::Query::Args::Args { name: String::new() };
                let _ = types::A_B::c::Args { name: String::new() };
                let _ = types::A::B_c::Args { name: String::new() };
                let _ = types::_self::_super::Args { _crate: String::new(), _Self: String::new() };
                let _ = types::__self::_super::Args { _crate: String::new(), _Self: String::new() };
            }
        "#;

        assert_consumer_compiles(&generated, consumer);
    }

    const HELLO_CONSUMER: &str = r#"
        pub struct Query;
        impl resolvers::QueryResolver<()> for Query {
            async fn hello<'a>(
                &'a self, _: &'a (), _: types::Query::hello::Args,
            ) -> Result<String, necrassrs::ResolverError> {
                Ok(String::from("Hello, Sheri"))
            }
        }
    "#;

    #[test]
    fn renamed_or_removed_field_rejects_existing_resolver_implementation() {
        for (sdl, compatible) in [
            ("type Query { hello: String! ping: String! }", true),
            ("type Query { greet: String! ping: String! }", false),
            ("type Query { ping: String! }", false),
        ] {
            let schema = Schema::parse_and_validate(sdl, "schema.graphql")
                .expect("the test schema must be valid");
            let generated = super::generate(&schema).expect("generation must succeed");
            if compatible {
                assert_consumer_compiles(&generated, HELLO_CONSUMER);
            } else {
                assert_consumer_fails(&generated, HELLO_CONSUMER, "E0407");
            }
        }
    }

    #[test]
    fn renamed_or_removed_argument_rejects_existing_argument_access() {
        let consumer = r#"
            pub struct Query;
            impl resolvers::QueryResolver<()> for Query {
                async fn hello<'a>(
                    &'a self, _: &'a (), args: types::Query::hello::Args,
                ) -> Result<String, necrassrs::ResolverError> {
                    Ok(format!("Hello, {}", args.name))
                }
            }
        "#;
        for (sdl, compatible) in [
            ("type Query { hello(name: String!): String! }", true),
            ("type Query { hello(who: String!): String! }", false),
            ("type Query { hello: String! }", false),
        ] {
            let schema = Schema::parse_and_validate(sdl, "schema.graphql")
                .expect("the test schema must be valid");
            let generated = super::generate(&schema).expect("generation must succeed");
            if compatible {
                assert_consumer_compiles(&generated, consumer);
            } else {
                assert_consumer_fails(&generated, consumer, "E0609");
            }
        }
    }

    #[test]
    fn added_field_preserves_existing_partial_implementation() {
        for sdl in [
            "type Query { hello: String! }",
            "type Query { hello: String! ping: String! }",
        ] {
            let schema = Schema::parse_and_validate(sdl, "schema.graphql")
                .expect("the test schema must be valid");
            let generated = super::generate(&schema).expect("generation must succeed");
            assert_consumer_compiles(&generated, HELLO_CONSUMER);
        }
    }

    fn assert_consumer_compiles(generated: &str, consumer: &str) {
        assert_consumer(generated, consumer, false);
    }

    fn assert_consumer_fails(generated: &str, consumer: &str, error_code: &str) {
        let output = compile_consumer(generated, consumer, None);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "expected compilation to fail with {error_code}"
        );
        let diagnostics: Vec<serde_json::Value> = stderr
            .lines()
            .map(|line| serde_json::from_str(line).expect("rustc must emit JSON diagnostics"))
            .collect();
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic["level"] == "error" && diagnostic["code"]["code"] == error_code
            }),
            "expected {error_code}, got:\n{stderr}",
        );
    }

    fn assert_consumer(generated: &str, consumer: &str, run: bool) {
        let executable = std::env::temp_dir().join(format!(
            "necrassrs-consumer-{}-{}{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            std::env::consts::EXE_SUFFIX,
        ));
        let output = compile_consumer(generated, consumer, run.then_some(executable.as_path()));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if run {
            let result = std::process::Command::new(&executable).output();
            std::fs::remove_file(&executable).unwrap();
            let output = result.expect("the compiled consumer must run");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn compile_consumer(
        generated: &str,
        consumer: &str,
        executable: Option<&std::path::Path>,
    ) -> std::process::Output {
        use std::{
            io::Write,
            path::Path,
            process::{Command, Stdio},
        };

        // ponytail: serialize shared Cargo artifact access; isolate target directories if throughput matters.
        static COMPILATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _compilation = COMPILATION
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        let build = Command::new(env!("CARGO"))
            .args([
                "build",
                "--locked",
                "--offline",
                "--message-format=json",
                "--manifest-path",
            ])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../necrassrs/Cargo.toml"))
            .output()
            .expect("Cargo must be available");
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let artifacts: Vec<_> = String::from_utf8(build.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|message| message["reason"] == "compiler-artifact")
            .collect();
        let mut compiler = Command::new("rustc");
        for name in ["necrassrs", "futures", "serde_json"] {
            let artifact = artifacts
                .iter()
                .find(|message| message["target"]["name"] == name)
                .expect("Cargo must report each consumer dependency");
            let library = artifact["filenames"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|name| name.as_str())
                .find(|name| name.ends_with(".rlib"))
                .expect("Cargo must report the library artifact");
            let directory = Path::new(library).parent().unwrap();
            compiler
                .arg("--extern")
                .arg(format!("{name}={library}"))
                .arg("-L")
                .arg(format!("dependency={}", directory.display()))
                .arg("-L")
                .arg(format!("dependency={}", directory.join("deps").display()));
        }
        if let Some(executable) = executable {
            compiler.arg("-o").arg(executable);
        } else {
            compiler.args(["--crate-type=lib", "--emit=metadata", "-o", "-"]);
        }
        let mut rustc = compiler
            .args([
                "--edition=2024",
                "--deny=warnings",
                "--error-format=json",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("rustc must be available");
        write!(rustc.stdin.take().unwrap(), "{generated}\n{consumer}").unwrap();
        rustc.wait_with_output().unwrap()
    }

    #[test]
    fn generated_field_identities_reuse_argument_types() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! ping: String! }",
            "schema.graphql",
        )
        .expect("the test schema must be valid");
        let generated = super::generate(&schema).expect("generation must succeed");
        let consumer = r#"
            use generated::{
                dispatch::SchemaDispatcher,
                fields,
                resolvers::QueryResolver,
                types,
            };
            use necrassrs::Field;

            struct Query;

            impl QueryResolver<()> for Query {}

            fn assert_field<F: Field<Args = types::Query::hello::Args>>() {}

            fn main() {
                assert_field::<fields::Query::hello>();

                let args = types::Query::hello::Args {
                    name: String::from("Sheri"),
                };
                assert_eq!(args.name, "Sheri");

                let _: <fields::Query::ping as Field>::Args =
                    types::Query::ping::Args {};

                let _ = generated::SDL;
                let _ = SchemaDispatcher::new(Query);
            }
        "#;

        assert_consumer(&format!("mod generated {{ {generated} }}"), consumer, true);
    }
}
