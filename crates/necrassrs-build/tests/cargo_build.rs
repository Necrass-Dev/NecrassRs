use quote::ToTokens;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
use syn::{ImplItem, ImplItemFn, Item, ItemImpl, ext::IdentExt, spanned::Spanned};

#[test]
fn first_build_creates_query_and_explicit_unimplemented_methods_from_sdl() {
    let consumer = Consumer::new(include_str!("fixtures/consumer/schema.graphql"));
    assert!(!consumer.resolvers().exists());
    consumer.bootstrap();
    let ast = consumer.ast();
    assert!(ast.items.iter().any(|item| matches!(
        item, Item::Struct(item) if item.ident == "Query"
    )));
    assert_eq!(method_names(&ast), ["hello", "ping"]);
    assert_stub(method(&ast, "hello"));
    assert_stub(method(&ast, "ping"));
}

#[test]
fn filling_generated_body_survives_regeneration_and_executes_without_sdl() {
    let consumer = Consumer::new(include_str!("fixtures/consumer/schema.graphql"));
    consumer.bootstrap();
    consumer.implement("hello", "Hello, Sheri");
    let source = fs::read(consumer.resolvers()).unwrap();

    // Force the build script to run; a cached build cannot prove preservation.
    let script = consumer.directory.join("build.rs");
    let mut contents = fs::read_to_string(&script).unwrap();
    contents.push('\n');
    fs::write(script, contents).unwrap();
    let build = consumer.build();
    assert_success(&build);
    assert_eq!(fs::read(consumer.resolvers()).unwrap(), source);

    let executable = messages(&build)
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
        .expect("Cargo must report the consumer executable");
    fs::remove_dir_all(consumer.directory.join("schema")).unwrap();
    let run = Command::new(executable)
        .current_dir(&consumer.directory)
        .env_clear()
        .env("PATH", "")
        .args([r#"{ hello(name: "Sheri") }"#, r#"{ hello(name: "Sheri") }"#])
        .output()
        .unwrap();
    assert_success(&run);
    let responses: Vec<serde_json::Value> = String::from_utf8(run.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        responses,
        vec![serde_json::json!({ "data": { "hello": "Hello, Sheri" } }); 2],
    );
}

#[test]
fn adding_sdl_field_in_nested_file_adds_stub_and_preserves_existing_body() {
    let consumer = Consumer::new(include_str!("fixtures/consumer/schema.graphql"));
    consumer.bootstrap();
    consumer.implement("hello", "retained hello");
    let before = body(method(&consumer.ast(), "hello"));
    let nested = consumer.directory.join("schema/query/fields");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        nested.join("extra.graphql"),
        "extend type Query { extra: String! }",
    )
    .unwrap();

    assert_success(&consumer.build());
    let ast = consumer.ast();
    assert_eq!(method_names(&ast), ["extra", "hello", "ping"]);
    assert_stub(method(&ast, "extra"));
    assert_eq!(body(method(&ast, "hello")), before);
}

#[test]
fn deleting_sdl_field_removes_its_implemented_method_and_preserves_others() {
    let consumer = Consumer::new(include_str!("fixtures/consumer/schema.graphql"));
    consumer.bootstrap();
    consumer.implement("hello", "retained hello");
    consumer.implement("ping", "deleted ping body");
    let before = body(method(&consumer.ast(), "hello"));
    consumer.schema("type Query { hello(name: String!): String! }");

    assert_success(&consumer.build());
    let ast = consumer.ast();
    assert_eq!(method_names(&ast), ["hello"]);
    assert_eq!(body(method(&ast, "hello")), before);
    assert!(
        !fs::read_to_string(consumer.resolvers())
            .unwrap()
            .contains("deleted ping body")
    );
}

#[test]
fn renaming_sdl_field_deletes_old_body_and_creates_a_new_stub() {
    let consumer = Consumer::new(include_str!("fixtures/consumer/schema.graphql"));
    consumer.bootstrap();
    consumer.implement("hello", "do not migrate this body");
    consumer.implement("ping", "retained ping");
    let before = body(method(&consumer.ast(), "ping"));
    consumer.schema("type Query { greet(name: String!): String! ping: String! }");

    assert_success(&consumer.build());
    let ast = consumer.ast();
    assert_eq!(method_names(&ast), ["greet", "ping"]);
    assert_stub(method(&ast, "greet"));
    assert_eq!(body(method(&ast, "ping")), before);
    assert!(
        !fs::read_to_string(consumer.resolvers())
            .unwrap()
            .contains("do not migrate this body")
    );
}

#[test]
fn modifying_arguments_updates_generated_contract_and_preserves_method_body() {
    let consumer = Consumer::new(include_str!("fixtures/consumer/schema.graphql"));
    consumer.bootstrap();
    consumer.implement("hello", "body independent of argument names");
    let before = body(method(&consumer.ast(), "hello"));
    consumer
        .schema("type Query { hello(greeting: String!, suffix: String!): String! ping: String! }");

    let build = consumer.build();
    assert_success(&build);
    let ast = consumer.ast();
    assert_eq!(method_names(&ast), ["hello", "ping"]);
    assert_eq!(body(method(&ast, "hello")), before);

    let output = messages(&build)
        .filter(|message| message["reason"] == "build-script-executed")
        .filter_map(|message| message["out_dir"].as_str().map(PathBuf::from))
        .map(|path| path.join("necrassrs.rs"))
        .find(|path| path.is_file())
        .expect("Cargo must report the generated contract in OUT_DIR");
    let generated = syn::parse_file(&fs::read_to_string(output).unwrap()).unwrap();
    let mut items = &generated.items;
    for name in ["types", "Query", "hello"] {
        items = items
            .iter()
            .find_map(|item| match item {
                Item::Mod(module) if module.ident.unraw() == name => {
                    module.content.as_ref().map(|(_, items)| items)
                }
                _ => None,
            })
            .expect("generated argument module must exist");
    }
    let args = items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "Args" => Some(item),
            _ => None,
        })
        .expect("generated Args must exist");
    let mut fields: Vec<_> = args
        .fields
        .iter()
        .map(|field| field.ident.as_ref().unwrap().unraw().to_string())
        .collect();
    fields.sort();
    assert_eq!(fields, ["greeting", "suffix"]);
}

#[test]
fn synchronization_uses_injective_names_without_matching_similar_methods() {
    let consumer = Consumer::new(
        "type Query { hello: String! Hello: String! type: String! self: String! _self: String! _: String! }",
    );
    consumer.bootstrap();
    let ast = consumer.ast();
    assert_eq!(
        method_names(&ast),
        ["Hello", "__", "__self", "_self", "hello", "type"]
    );
    method_names(&ast)
        .iter()
        .for_each(|name| assert_stub(method(&ast, name)));
    consumer.implement("__self", "keep underscore-prefixed field");
    consumer.implement("Hello", "keep case-sensitive field");
    let before = consumer.ast();
    consumer.schema("type Query { Hello: String! type: String! _self: String! _: String! }");

    assert_success(&consumer.build());
    let after = consumer.ast();
    assert_eq!(method_names(&after), ["Hello", "__", "__self", "type"]);
    for name in ["__self", "Hello"] {
        assert_eq!(body(method(&after, name)), body(method(&before, name)));
    }
}

#[test]
fn invalid_sdl_does_not_destroy_existing_implementation() {
    let consumer = Consumer::new(include_str!("fixtures/consumer/schema.graphql"));
    consumer.bootstrap();
    consumer.implement("hello", "preserve through failed synchronization");
    let before = fs::read(consumer.resolvers()).unwrap();
    consumer.schema("type Query { hello(name:): String! }");

    assert!(!consumer.build().status.success());
    assert_eq!(fs::read(consumer.resolvers()).unwrap(), before);
}

#[test]
fn unchanged_sdl_preserves_equivalent_return_type_spelling_and_comments() {
    let consumer = Consumer::new("type Query { hello(name: String!): String! }");
    consumer.bootstrap();
    consumer.implement("hello", "retained hello");
    let ast = consumer.ast();
    let mut source = fs::read_to_string(consumer.resolvers()).unwrap();
    source.replace_range(
        method(&ast, "hello").sig.output.span().byte_range(),
        "-> Result</* Keep this return contract comment. */ String, necrassrs::ResolverError>",
    );
    fs::write(consumer.resolvers(), &source).unwrap();

    assert_success(&consumer.build());
    assert_eq!(
        fs::read_to_string(consumer.resolvers()).unwrap(),
        source,
        "an unchanged SDL contract must preserve equivalent type spelling and comments",
    );
}

#[test]
fn added_method_does_not_shadow_existing_impl_lifetime() {
    let consumer = Consumer::new("type Query { hello(name: String!): String! }");
    consumer.bootstrap();
    consumer.implement("hello", "retained hello");
    let mut ast = consumer.ast();
    let before = body(method(&ast, "hello"));
    for item in &mut ast.items {
        let Item::Impl(implementation) = item else {
            continue;
        };
        implementation.generics = syn::parse_quote!(<'a>);
        implementation.trait_.as_mut().unwrap().1 =
            syn::parse_quote!(crate::generated::resolvers::QueryResolver<Context<'a>>);
        for item in &mut implementation.items {
            if let ImplItem::Fn(method) = item {
                method.sig = syn::parse_quote! {
                    async fn hello<'call>(
                        &'call self,
                        _context: &'call Context<'a>,
                        _args: crate::generated::types::Query::hello::Args,
                    ) -> ::core::result::Result<::std::string::String, ::necrassrs::ResolverError>
                };
            }
        }
    }
    ast.items.push(syn::parse_quote!(
        pub struct Context<'a>(pub &'a str);
    ));
    fs::write(consumer.resolvers(), ast.to_token_stream().to_string()).unwrap();
    fs::write(
        consumer.directory.join("src/main.rs"),
        r#"
mod generated { include!(concat!(env!("OUT_DIR"), "/necrassrs.rs")); }
mod resolvers;

fn main() {
    fn require_resolver<C, Q: generated::resolvers::QueryResolver<C>>(_: &C, _: &Q) {}
    let name = String::from("Sheri");
    let context = resolvers::Context(&name);
    require_resolver(&context, &resolvers::Query);
}
"#,
    )
    .unwrap();
    // Establish that the customized implementation compiles before adding a field.
    assert_success(&consumer.build());
    consumer.schema("type Query { hello(name: String!): String! extra: String! }");

    assert_success(&consumer.build());
    let ast = consumer.ast();
    assert_eq!(method_names(&ast), ["extra", "hello"]);
    assert_stub(method(&ast, "extra"));
    assert_eq!(body(method(&ast, "hello")), before);
}

#[test]
fn adding_field_to_bom_prefixed_source_preserves_existing_code() {
    let consumer = Consumer::new("type Query { hello(name: String!): String! }");
    consumer.bootstrap();
    consumer.implement("hello", "retained hello");
    let source = fs::read_to_string(consumer.resolvers()).unwrap();
    let ast = syn::parse_file(&source).unwrap();
    let existing_method = &source[method(&ast, "hello").span().byte_range()];
    fs::write(consumer.resolvers(), format!("\u{feff}{source}")).unwrap();
    // A BOM is valid Rust input and must not prevent an unchanged build.
    assert_success(&consumer.build());
    consumer.schema("type Query { hello(name: String!): String! extra: String! }");

    assert_success(&consumer.build());
    let updated = fs::read_to_string(consumer.resolvers()).unwrap();
    assert!(updated.starts_with('\u{feff}'));
    assert!(updated.contains(existing_method));
    let ast = syn::parse_file(&updated).unwrap();
    assert_eq!(method_names(&ast), ["extra", "hello"]);
    assert_stub(method(&ast, "extra"));
}

fn resolver_impl(ast: &syn::File) -> &ItemImpl {
    ast.items
        .iter()
        .find_map(|item| match item {
            Item::Impl(item)
                if item.trait_.as_ref().is_some_and(|(_, path, _)| {
                    path.segments
                        .last()
                        .is_some_and(|segment| segment.ident == "QueryResolver")
                }) =>
            {
                Some(item)
            }
            _ => None,
        })
        .expect("the source file must contain an explicit QueryResolver implementation")
}

fn method_names(ast: &syn::File) -> Vec<String> {
    let mut names: Vec<_> = resolver_impl(ast)
        .items
        .iter()
        .filter_map(|item| match item {
            ImplItem::Fn(method) => Some(method.sig.ident.unraw().to_string()),
            _ => None,
        })
        .collect();
    names.sort();
    names
}

fn method<'a>(ast: &'a syn::File, name: &str) -> &'a ImplItemFn {
    resolver_impl(ast)
        .items
        .iter()
        .find_map(|item| match item {
            ImplItem::Fn(method) if method.sig.ident.unraw() == name => Some(method),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing explicit resolver method {name}"))
}

fn body(method: &ImplItemFn) -> String {
    method.block.to_token_stream().to_string()
}

fn assert_stub(method: &ImplItemFn) {
    assert!(
        method.sig.asyncness.is_some(),
        "editable resolver methods must be async"
    );
    assert!(
        method.block.stmts.iter().any(|statement| {
            let path = match statement {
                syn::Stmt::Macro(statement) => &statement.mac.path,
                syn::Stmt::Expr(syn::Expr::Macro(expression), _) => &expression.mac.path,
                _ => return false,
            };
            path.segments
                .last()
                .is_some_and(|segment| segment.ident == "unimplemented")
        }),
        "new methods must have an explicit unimplemented!() body"
    );
}

fn messages(output: &Output) -> impl Iterator<Item = serde_json::Value> + '_ {
    std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Consumer {
    directory: PathBuf,
}

impl Consumer {
    fn new(sdl: &str) -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let directory = std::env::temp_dir().join(format!(
            "necrassrs-sync-consumer-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&directory).unwrap();
        let consumer = Self { directory };
        fs::create_dir(consumer.directory.join("src")).unwrap();
        fs::create_dir(consumer.directory.join("schema")).unwrap();
        let runtime = workspace.join("crates/necrassrs").canonicalize().unwrap();
        let build_library = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        let package_name = consumer.directory.file_name().unwrap().to_str().unwrap();
        fs::write(
            consumer.directory.join("Cargo.toml"),
            format!(
                r#"
            [package]
            name = "{package_name}"
            version = "0.0.0"
            edition = "2024"
            [workspace]
            [dependencies]
            necrassrs = {{ path = {runtime:?} }}
            futures = "0.3"
            serde_json = "1.0"
            [build-dependencies]
            necrassrs-build = {{ path = {build_library:?} }}
            miette = "7.6.0"
        "#
            ),
        )
        .unwrap();
        fs::copy(
            workspace.join("Cargo.lock"),
            consumer.directory.join("Cargo.lock"),
        )
        .unwrap();
        fs::write(
            consumer.directory.join("build.rs"),
            include_str!("fixtures/consumer/build.rs"),
        )
        .unwrap();
        fs::write(
            consumer.directory.join("src/main.rs"),
            include_str!("fixtures/consumer/main.rs"),
        )
        .unwrap();
        consumer.schema(sdl);
        consumer
    }

    fn schema(&self, sdl: &str) {
        fs::write(self.directory.join("schema/schema.graphql"), sdl).unwrap();
    }

    fn resolvers(&self) -> PathBuf {
        self.directory.join("src/resolvers.rs")
    }

    fn ast(&self) -> syn::File {
        syn::parse_file(&fs::read_to_string(self.resolvers()).unwrap()).unwrap()
    }

    fn bootstrap(&self) {
        let build = self.build();
        assert!(
            self.resolvers().is_file(),
            "build.rs must create src/resolvers.rs from SDL; the test supplies no Query implementation\n{}",
            String::from_utf8_lossy(&build.stderr)
        );
        assert_success(&build);
    }

    fn implement(&self, name: &str, value: &str) {
        let mut ast = self.ast();
        let implementation = ast
            .items
            .iter_mut()
            .find_map(|item| match item {
                Item::Impl(item)
                    if item.trait_.as_ref().is_some_and(|(_, path, _)| {
                        path.segments
                            .last()
                            .is_some_and(|segment| segment.ident == "QueryResolver")
                    }) =>
                {
                    Some(item)
                }
                _ => None,
            })
            .unwrap();
        let method = implementation
            .items
            .iter_mut()
            .find_map(|item| match item {
                ImplItem::Fn(method) if method.sig.ident.unraw() == name => Some(method),
                _ => None,
            })
            .unwrap();
        method.block = syn::parse_quote!({ Ok(::std::string::String::from(#value)) });
        fs::write(self.resolvers(), ast.to_token_stream().to_string()).unwrap();
    }

    fn build(&self) -> Output {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Command::new(env!("CARGO"))
            .current_dir(&self.directory)
            .env("NO_COLOR", "1")
            .env("CARGO_TERM_COLOR", "never")
            .args([
                "build",
                "--offline",
                "--message-format=json",
                "--target-dir",
            ])
            .arg(workspace.join("target/cargo-consumer"))
            .output()
            .expect("Cargo must be available")
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
