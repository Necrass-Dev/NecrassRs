use apollo_compiler::{
    Schema, ast::Type, parser::SourceSpan, schema::ExtendedType, validation::Valid,
};
use quote::{format_ident, quote};

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
    let mut object_modules = Vec::new();

    for (type_name, definition) in &schema.types {
        if type_name.as_str().starts_with("__") {
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
                    let argument_type = match argument.ty.as_ref() {
                        Type::NonNullNamed(name) if name.as_str() == "String" => {
                            quote! { ::std::string::String }
                        }
                        unsupported => {
                            return Err(CodegenError::new(
                                format!(
                                    "Unsupported argument type at {type_name}.{field_name}({}): {unsupported}",
                                    argument.name,
                                ),
                                schema,
                                argument.ty.location(),
                            ));
                        }
                    };

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
            let return_type = match &field.ty {
                Type::NonNullNamed(name) if name.as_str() == "String" => {
                    quote! { ::std::string::String }
                }
                unsupported => {
                    return Err(CodegenError::new(
                        format!(
                            "Unsupported return type at {type_name}.{field_name}: {unsupported}"
                        ),
                        schema,
                        field.ty.inner_named_type().location(),
                    ));
                }
            };
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
        let arguments = field.arguments.iter().map(|argument| {
            let name = argument.name.as_str();
            let member = format_ident!("r#{}", rust_name(name));
            let message = format!("Expected a String argument at {type_name}.{field_name}({name})");
            quote! {
                #member: _arguments.get(#name)
                    .and_then(::necrassrs::JsonValue::as_str)
                    .ok_or_else(|| ::necrassrs::ResolverError::new(#message))?
                    .to_owned(),
            }
        });
        branches.push(quote! {
            (#type_name, #field_name) => {
                let args = super::types::#object_name::#method_name::Args {
                    #(#arguments)*
                };
                super::resolvers::#resolver_name::#method_name(&self.query, context, args)
                    .await
                    .map(::necrassrs::JsonValue::from)
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
                ) -> ::core::result::Result<::necrassrs::JsonValue, ::necrassrs::ResolverError> {
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

fn rust_name(name: &str) -> String {
    if name.starts_with('_') || matches!(name, "self" | "Self" | "super" | "crate") {
        format!("_{name}")
    } else {
        name.to_owned()
    }
}

#[derive(Debug, miette::Diagnostic)]
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

#[cfg(test)]
mod test {
    use apollo_compiler::Schema;
    use miette::Diagnostic;

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
