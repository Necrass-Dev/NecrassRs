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
/// objects, lists, and nullable wrappers. Custom scalars and composite output
/// types are not yet supported.
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
    let resolvers = generate_resolvers(schema)?;
    let dispatch = generate_dispatch(schema)?;
    let sdl = schema.to_string();

    Ok(quote! {
        pub const SDL: &str = #sdl;

        #types
        #resolvers
        #dispatch
    }
    .to_string())
}

fn generate_types(schema: &Valid<Schema>) -> Result<impl quote::ToTokens, CodegenError> {
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

            let (fields, field_values) = input_object
                .fields
                .iter()
                .map(|(field_name, field)| {
                    let graphql_name = field_name.as_str();
                    let member = format_ident!("r#{}", rust_name(graphql_name));
                    let field_type = input_type(schema, field.ty.as_ref(), &quote! { self })
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
                    let coordinate = format!("{type_name}.{field_name}");
                    let lookup = quote! { object.get(#graphql_name) };
                    let field_value = input_position_value(
                        schema,
                        field.ty.as_ref(),
                        &lookup,
                        &coordinate,
                        &quote! { self },
                    )
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
                    pub(super) fn from_graphql_value(
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
        #[allow(non_snake_case)]
        pub mod types {
            #(#named_types)*
            #(#object_modules)*
        }
    })
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
            let method_name = format_ident!("r#{}", rust_name(field_name.as_str()));
            let return_type = resolver_return_type(schema, type_name, field_name, &field.ty)?;
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
            #[allow(non_camel_case_types, non_snake_case)]
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
    let mut branches = Vec::new();

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
                let value = super::resolvers::#resolver_name::#method_name(&self.query, context, args)
                    .await?;
                Ok(#conversion)
            }
        });
    }

    Ok(quote! {
        pub mod dispatch {
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
                Q: super::resolvers::#resolver_name<C> + ::core::marker::Sync,
            {
                async fn resolve<'a>(
                    &'a self,
                    context: &'a C,
                    coordinate: ::necrassrs::FieldCoordinate<'a>,
                    _arguments: &'a ::necrassrs::JsonMap,
                ) -> ::core::result::Result<::necrassrs::ResolvedValue, ::necrassrs::ResolverError> {
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
) -> Result<TokenStream, CodegenError> {
    output_type(schema, ty, &quote! { super::types }).ok_or_else(|| {
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
                    <i32 as ::core::convert::TryFrom<i64>>::try_from(value).ok()
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
            (#value).as_str()
                .map(::std::borrow::ToOwned::to_owned)
                .or_else(|| {
                    (#value).as_i64().map(|value| value.to_string())
                })
                .map(::necrassrs::Id::from)
                .ok_or_else(|| ::necrassrs::ResolverError::new(#message))?
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
                    #types_path::#name::from_graphql_value(#value)?
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
                    let value: f64 = #value;

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
        "Int" => Some(quote! { i32 }),
        "Float" => Some(quote! { f64 }),
        "String" => Some(quote! { ::std::string::String }),
        "Boolean" => Some(quote! { bool }),
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
            let mut items = ::std::vec::Vec::<::necrassrs::ResolvedValue>::new();

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
            ("intResult", quote! { i32 }),
            ("floatResult", quote! { f64 }),
            ("stringResult", quote! { ::std::string::String }),
            ("booleanResult", quote! { bool }),
            ("idResult", quote! { ::necrassrs::Id }),
            ("statusResult", quote! { super::types::r#Status }),
            (
                "nullableStatusResult",
                quote! { ::core::option::Option<super::types::r#Status> },
            ),
            (
                "nullableListResult",
                quote! { ::core::option::Option<::std::vec::Vec<::core::option::Option<i32>>> },
            ),
            (
                "requiredListResult",
                quote! { ::std::vec::Vec<::core::option::Option<i32>> },
            ),
            (
                "nullableNonNullItemsResult",
                quote! { ::core::option::Option<::std::vec::Vec<i32>> },
            ),
            (
                "requiredNonNullItemsResult",
                quote! { ::std::vec::Vec<i32> },
            ),
        ];

        for (field_name, expected) in cases {
            let field = &query.fields[field_name];
            let actual =
                super::resolver_return_type(&schema, query.name.as_str(), field_name, &field.ty)
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
            "fn from_graphql_value",
            "as_object",
            "object . get",
            "pub r#required : bool",
            "pub r#optionalId : :: necrassrs :: GraphQLInput < :: necrassrs :: Id >",
            "pub r#statuses : :: necrassrs :: GraphQLInput",
            "self :: r#Nested :: from_graphql_value",
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
}
