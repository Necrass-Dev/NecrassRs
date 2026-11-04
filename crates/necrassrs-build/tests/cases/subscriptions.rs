use super::*;

const EMPTY_MAIN: &str = r#"
mod generated { include!(concat!(env!("OUT_DIR"), "/necrassrs.rs")); }
mod resolvers;
fn main() {}
"#;

#[test]
fn subscription_fixture_syntax_is_valid() {
    for source in [
        include_str!("../fixtures/subscription_runtime.rs"),
        include_str!("../fixtures/subscription/resolvers.rs"),
        include_str!("../fixtures/subscription/scalars.rs"),
        include_str!("../fixtures/subscription/main.rs"),
    ] {
        syn::parse_file(source).expect("consumer fixture must be valid Rust syntax");
    }
    reed_compiler::Schema::parse_and_validate(
        include_str!("../fixtures/subscription/schema.graphql"),
        "subscription.graphql",
    )
    .expect("consumer fixture must have a valid schema");
}

fn test_consumer(consumer: &Consumer) -> Output {
    Command::new(env!("CARGO"))
        .current_dir(&consumer.directory)
        .env("CARGO_TERM_COLOR", "never")
        .args(["test", "--offline", "--target-dir"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/cargo-consumer"))
        .output()
        .expect("Cargo must be available")
}

fn subscription_consumer(sdl: &str) -> Consumer {
    let consumer = Consumer::new(sdl);
    fs::write(consumer.directory.join("src/main.rs"), EMPTY_MAIN).unwrap();
    consumer
}

fn subscriber<'a>(ast: &'a syn::File, root: &str, field: &str) -> &'a ItemImpl {
    ast.items
        .iter()
        .find_map(|item| {
            let Item::Impl(item) = item else { return None };
            let (_, path, _) = item.trait_.as_ref()?;
            let last = path.segments.last()?;
            if last.ident != "Subscriber" {
                return None;
            }
            let PathArguments::AngleBracketed(args) = &last.arguments else {
                return None;
            };
            let GenericArgument::Type(Type::Path(marker)) = args.args.first()? else {
                return None;
            };
            let mut parts = marker.path.segments.iter().rev();
            (parts.next()?.ident.unraw() == field && parts.next()?.ident.unraw() == root)
                .then_some(item)
        })
        .unwrap_or_else(|| panic!("missing Subscriber for {root}.{field}"))
}

fn resolve(implementation: &ItemImpl) -> &ImplItemFn {
    implementation
        .items
        .iter()
        .find_map(|item| match item {
            ImplItem::Fn(method) if method.sig.ident == "resolve" => Some(method),
            _ => None,
        })
        .expect("Subscriber must use resolve, not subscribe")
}

fn implement_empty_source(consumer: &Consumer, field: &str) -> String {
    let ast = consumer.ast();
    let range = resolve(subscriber(&ast, "Subscription", field))
        .block
        .span()
        .byte_range();
    let mut source = fs::read_to_string(consumer.resolvers()).unwrap();
    source.replace_range(
        range,
        "{ Ok(necrassrs::SourceStream::from_stream(futures::stream::empty())) }",
    );
    fs::write(consumer.resolvers(), &source).unwrap();
    source
}

#[test]
fn subscription_runtime_public_contract_and_lifecycle() {
    let consumer = Consumer::new("type Query { ping: Int! }");
    // Exercise runtime contracts even while subscription generation is unsupported.
    fs::write(consumer.directory.join("build.rs"), "fn main() {}\n").unwrap();
    fs::write(consumer.directory.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::create_dir(consumer.directory.join("tests")).unwrap();
    fs::write(
        consumer.directory.join("tests/subscriptions.rs"),
        include_str!("../fixtures/subscription_runtime.rs"),
    )
    .unwrap();
    assert_success(&test_consumer(&consumer));
}

#[test]
fn subscription_bootstrap_creates_resolve_stubs_and_root_constructors() {
    for mutation in [false, true] {
        let mutation_sdl = if mutation {
            "type Mutation { update: Int! }"
        } else {
            ""
        };
        let consumer = subscription_consumer(&format!(
            "type Query {{ ping: Int! }} {mutation_sdl} type Subscription {{ orderChanged: String! }}"
        ));
        consumer.bootstrap();
        let ast = consumer.ast();
        assert!(
            root_resolver_impl(&ast, "SubscriptionResolver")
                .items
                .is_empty()
        );
        let field = subscriber(&ast, "Subscription", "orderChanged");
        assert_stub(resolve(field));
        assert!(
            !field.items.iter().any(
                |item| matches!(item, ImplItem::Fn(method) if method.sig.ident == "subscribe")
            )
        );
        assert!(
            output_type(field)
                .to_token_stream()
                .to_string()
                .ends_with("String")
        );
        let roots = if mutation {
            "resolvers::Query, resolvers::Mutation, resolvers::Subscription"
        } else {
            "resolvers::Query, resolvers::Subscription"
        };
        fs::write(
            consumer.directory.join("src/main.rs"),
            EMPTY_MAIN.replace(
                "fn main() {}",
                &format!(
                    "fn main() {{ let _ = generated::dispatch::SchemaDispatcher::new({roots}); }}"
                ),
            ),
        )
        .unwrap();
        assert_success(&consumer.build());
        let before = fs::read(consumer.resolvers()).unwrap();
        assert_success(&consumer.build());
        assert_eq!(before, fs::read(consumer.resolvers()).unwrap());
    }
}

#[test]
fn subscription_generated_consumer_executes_typed_and_abstract_events() {
    let consumer = subscription_consumer(include_str!("../fixtures/subscription/schema.graphql"));
    consumer.bootstrap();
    fs::write(
        consumer.resolvers(),
        include_str!("../fixtures/subscription/resolvers.rs"),
    )
    .unwrap();
    fs::write(
        consumer.directory.join("src/resolvers/scalars.rs"),
        include_str!("../fixtures/subscription/scalars.rs"),
    )
    .unwrap();
    fs::write(
        consumer.directory.join("src/main.rs"),
        include_str!("../fixtures/subscription/main.rs"),
    )
    .unwrap();
    assert_success(&test_consumer(&consumer));
}

#[test]
fn subscription_sync_preserves_bodies_updates_output_and_adds_or_removes_stubs() {
    let consumer = subscription_consumer(
        "type Query { ping: Int! } type Subscription { kept: String! removed: Int! }",
    );
    consumer.bootstrap();
    implement_empty_source(&consumer, "kept");
    let retained = body(resolve(subscriber(&consumer.ast(), "Subscription", "kept")));
    consumer.schema("type Query { ping: Int! } type Subscription { kept: [String!]! added: Int! }");
    assert_success(&consumer.build());
    let ast = consumer.ast();
    let kept = subscriber(&ast, "Subscription", "kept");
    assert_eq!(body(resolve(kept)), retained);
    let output = output_type(kept).to_token_stream().to_string();
    assert!(output.contains("Vec") && output.contains("String"));
    assert_stub(resolve(subscriber(&ast, "Subscription", "added")));
    let source = fs::read_to_string(consumer.resolvers()).unwrap();
    assert!(!source.contains("::removed"));
    assert_success(&consumer.build());
    assert_eq!(source, fs::read_to_string(consumer.resolvers()).unwrap());
}

#[test]
fn subscription_sync_renames_untouched_stubs_without_migrating_bodies() {
    let consumer =
        subscription_consumer("type Query { ping: Int! } type Subscription { oldName: Int! }");
    consumer.bootstrap();
    consumer.schema("type Query { ping: Int! } type Subscription { newName: Int! }");
    assert_success(&consumer.build());
    assert_stub(resolve(subscriber(
        &consumer.ast(),
        "Subscription",
        "newName",
    )));
    assert!(
        !fs::read_to_string(consumer.resolvers())
            .unwrap()
            .contains("::oldName")
    );
}

#[test]
fn subscription_sync_refuses_deleting_or_renaming_edited_fields_before_writes() {
    for replacement in ["other: Int!", "newName: String! other: Int!"] {
        let consumer = subscription_consumer(
            "type Query { ping: Int! } type Subscription { oldName: String! }",
        );
        consumer.bootstrap();
        let before = implement_empty_source(&consumer, "oldName");
        consumer.schema(&format!(
            "type Query {{ ping: Int! }} type Subscription {{ {replacement} }}"
        ));
        let build = consumer.build();
        assert!(
            !build.status.success(),
            "edited declaration was silently discarded"
        );
        assert_eq!(before, fs::read_to_string(consumer.resolvers()).unwrap());
    }
}

#[test]
fn subscription_sync_updates_moved_implementations_in_place() {
    let consumer =
        subscription_consumer("type Query { ping: Int! } type Subscription { kept: String! }");
    consumer.bootstrap();
    let ast = consumer.ast();
    let range = subscriber(&ast, "Subscription", "kept").span().byte_range();
    let mut entry = fs::read_to_string(consumer.resolvers()).unwrap();
    let moved = entry[range.clone()].replace("self::Subscription", "super::Subscription");
    entry.replace_range(range, "mod events;");
    fs::create_dir_all(consumer.directory.join("src/resolvers")).unwrap();
    let path = consumer.directory.join("src/resolvers/events.rs");
    fs::write(&path, moved).unwrap();
    fs::write(consumer.resolvers(), &entry).unwrap();
    consumer.schema("type Query { ping: Int! } type Subscription { kept: [String!]! }");
    assert_success(&consumer.build());
    assert_eq!(entry, fs::read_to_string(consumer.resolvers()).unwrap());
    let moved = syn::parse_file(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(
        output_type(subscriber(&moved, "Subscription", "kept"))
            .to_token_stream()
            .to_string()
            .contains("Vec")
    );
}

#[test]
fn subscription_sync_refuses_removed_arguments_used_by_retained_body() {
    let consumer = subscription_consumer(
        "type Query { ping: Int! } type Subscription { kept(id: ID!): String! }",
    );
    consumer.bootstrap();
    let ast = consumer.ast();
    let method = resolve(subscriber(&ast, "Subscription", "kept"));
    let syn::FnArg::Typed(argument) = method.sig.inputs.last().unwrap() else {
        panic!("expected Args parameter");
    };
    let syn::Pat::Ident(binding) = argument.pat.as_ref() else {
        panic!("expected named Args parameter");
    };
    let mut before = fs::read_to_string(consumer.resolvers()).unwrap();
    before.replace_range(method.block.span().byte_range(), &format!(
        "{{ let _id = {}.id; Ok(necrassrs::SourceStream::from_stream(futures::stream::empty())) }}",
        binding.ident,
    ));
    fs::write(consumer.resolvers(), &before).unwrap();
    consumer.schema("type Query { ping: Int! } type Subscription { kept: String! added: Int! }");
    assert!(!consumer.build().status.success());
    assert_eq!(before, fs::read_to_string(consumer.resolvers()).unwrap());
}

#[test]
fn subscription_bootstrap_reuses_custom_root_context_and_injective_names() {
    let consumer = subscription_consumer(
        "schema { query: Root subscription: Feed } type Root { ping: Int! } \
         type Feed { type(self: String!, _self: String!): String! orderChanged: Int! order_changed: Int! }",
    );
    fs::write(consumer.resolvers(),
        "pub struct AppContext; pub struct AppQuery; impl crate::generated::resolvers::RootResolver<AppContext> for AppQuery {}\n").unwrap();
    consumer.bootstrap();
    let ast = consumer.ast();
    assert!(
        root_resolver_impl(&ast, "FeedResolver")
            .to_token_stream()
            .to_string()
            .contains("AppContext")
    );
    for name in ["type", "orderChanged", "order_changed"] {
        assert_stub(resolve(subscriber(&ast, "Feed", name)));
    }
    fs::write(
        consumer.directory.join("src/main.rs"),
        EMPTY_MAIN.replace(
            "fn main() {}",
            r#"
fn main() {
    let _ = generated::dispatch::SchemaDispatcher::new(resolvers::AppQuery, resolvers::Feed);
    let args = generated::types::Feed::r#type::Args { _self: "one".into(), __self: "two".into() };
    assert_eq!(args._self, "one");
    assert_eq!(args.__self, "two");
}
"#,
        ),
    )
    .unwrap();
    assert_success(&consumer.build());
}
