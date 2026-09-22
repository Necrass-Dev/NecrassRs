use apollo_compiler::{Schema, ast::Type, schema::ExtendedType, validation::Valid};
use quote::{format_ident, quote};

pub fn generate(schema: &Valid<Schema>) -> Result<String, CodegenError> {
    let types = generate_types(schema)?;
    let resolvers = generate_resolvers(schema)?;
    let dispatch = generate_dispatch(schema)?;

    Ok(quote! {
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
                            return Err(CodegenError {
                                message: format!(
                                    "Unsupported argument type at {type_name}.{field_name}({}): {unsupported}",
                                    argument.name,
                                ),
                            });
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

        if !field_modules.is_empty() {
            object_modules.push(quote! {
                pub mod #object_name {
                    #(#field_modules)*
                }
            });
        }
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
                    return Err(CodegenError {
                        message: format!(
                            "Unsupported return type at {type_name}.{field_name}: {unsupported}"
                        ),
                    });
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
    if schema.schema_definition.mutation.is_some()
        || schema.schema_definition.subscription.is_some()
    {
        return Err(CodegenError {
            message: "Generated dispatch currently supports query-only schemas".to_owned(),
        });
    }
    let query = schema
        .schema_definition
        .query
        .as_ref()
        .and_then(|name| schema.get_object(name.as_str()))
        .ok_or_else(|| CodegenError {
            message: "A query root object is required".to_owned(),
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

#[derive(Debug)]
pub struct CodegenError {
    message: String,
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

    #[test]
    fn generates_required_string_argument_struct() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .expect("the test schema must be valid");

        let generated = super::generate(&schema).expect("generation must succeed");
        let normalized: String = generated.split_whitespace().collect();

        assert!(
            normalized.contains("pubmodr#Query{pubmodr#hello{pubstructArgs{pubr#name:"),
            "expected a public argument struct in the types module, got:\n{generated}"
        );
    }

    #[test]
    fn resolver_trait_names_preserve_object_names() {
        let schema = Schema::parse_and_validate(
            r#"
                type Query { hello(name: String!): String! }
                type User { hello(name: String!): String! }
                type user { hello(name: String!): String! }
                type UserResolver { hello(name: String!): String! }
            "#,
            "schema.graphql",
        )
        .expect("the test schema must be valid");

        let generated = super::generate(&schema).expect("generation must succeed");
        let normalized = generated
            .split_whitespace()
            .collect::<String>()
            .replace("r#", "");

        assert!(
            normalized.contains("pubmodresolvers{"),
            "expected a public resolvers module, got:\n{generated}"
        );
        for name in [
            "QueryResolver",
            "UserResolver",
            "userResolver",
            "UserResolverResolver",
        ] {
            assert_eq!(
                normalized.matches(&format!("pubtrait{name}<")).count(),
                1,
                "expected exactly one public {name} trait with a Context parameter, got:\n{generated}"
            );
        }
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
    fn generated_dispatch_executes_hello() {
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
                    "type Query { hello(name: String!): String! }", "schema.graphql",
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
                let schema = necrassrs::Schema::parse_and_validate(SDL, "schema.graphql").unwrap();
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
        let source = format!("const SDL: &str = {sdl:?}; mod generated {{ {generated} }}");
        assert_consumer(&source, consumer, true);
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

    fn assert_consumer_compiles(generated: &str, consumer: &str) {
        assert_consumer(generated, consumer, false);
    }

    fn assert_consumer(generated: &str, consumer: &str, run: bool) {
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
        let executable = std::env::temp_dir().join(format!(
            "necrassrs-consumer-{}-{}{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            std::env::consts::EXE_SUFFIX,
        ));
        if run {
            compiler.arg("-o").arg(&executable);
        } else {
            compiler.args(["--crate-type=lib", "--emit=metadata", "-o", "-"]);
        }
        let mut rustc = compiler
            .args(["--edition=2024", "--deny=warnings", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("rustc must be available");
        write!(rustc.stdin.take().unwrap(), "{generated}\n{consumer}").unwrap();
        let output = rustc.wait_with_output().unwrap();
        let execution = if run && output.status.success() {
            let result = Command::new(&executable).output();
            std::fs::remove_file(&executable).unwrap();
            Some(result.expect("the compiled consumer must run"))
        } else {
            None
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if let Some(output) = execution {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
