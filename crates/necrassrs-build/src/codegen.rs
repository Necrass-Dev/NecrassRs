use apollo_compiler::{Schema, ast::Type, schema::ExtendedType, validation::Valid};
use quote::{format_ident, quote};

pub fn generate(schema: &Valid<Schema>) -> Result<String, CodegenError> {
    let types = generate_types(schema)?;
    let resolvers = generate_resolvers(schema);

    Ok(quote! {
        #types
        #resolvers
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
            if field.arguments.is_empty() {
                continue;
            }

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

fn generate_resolvers(schema: &Valid<Schema>) -> impl quote::ToTokens {
    let mut resolvers = Vec::new();

    for (type_name, definition) in &schema.types {
        if type_name.as_str().starts_with("__") || !matches!(definition, ExtendedType::Object(_)) {
            continue;
        }

        let resolver_name = format_ident!("{}Resolver", rust_name(type_name.as_str()));
        resolvers.push(quote! {
            #[allow(non_camel_case_types)]
            pub trait #resolver_name<C> {}
        });
    }

    quote! {
        pub mod resolvers {
            #(#resolvers)*
        }
    }
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
    fn generated_paths_preserve_case_boundaries_and_escape_rust_names() {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };

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

        let mut rustc = Command::new("rustc")
            .args([
                "--edition=2024",
                "--crate-type=lib",
                "--emit=metadata",
                "--deny=warnings",
                "-o",
                "-",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("rustc must be available");
        write!(rustc.stdin.take().unwrap(), "{generated}\n{consumer}").unwrap();
        let output = rustc.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
