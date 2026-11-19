//! Deeply composed request documents fail as request errors instead of overflowing the stack.

use necrassrs::{
    Dispatcher, ExecutionOptions, FieldCoordinate, JsonMap, Request, ResolvedValue, ResolverError,
};
use reed_compiler::Schema;
use std::process::Command;

const STACK_BYTES: usize = 2 * 1024 * 1024;
const CHILD_ENV: &str = "NECRASSRS_REQUEST_DEPTH_CHILD";
const CHILD_MARKER: &str = "REQUEST_DEPTH_CHILD_RAN";

struct NoDispatch;

impl Dispatcher<()> for NoDispatch {
    async fn resolve<'a>(
        &'a self,
        _: &'a (),
        _: FieldCoordinate<'a>,
        _: &'a JsonMap,
    ) -> Result<ResolvedValue<()>, ResolverError> {
        panic!("rejected requests must fail before application dispatch");
    }
}

/// Ten introspection fragments of 470 `ofType` levels each: about 14k tokens, within the
/// default token limit, with a composed selection depth above 4,700. Whitespace also counts
/// toward the token limit, so the document is written without it.
fn deep_of_type_document() -> String {
    let mut document = String::from("query{__schema{types{...f1}}}");
    for i in 1..=10 {
        let inner = if i < 10 {
            format!("...f{}", i + 1)
        } else {
            "name".to_string()
        };
        document.push_str(&format!(
            "\nfragment f{i} on __Type{{{}{inner}{}}}",
            "ofType{".repeat(470),
            "}".repeat(470)
        ));
    }
    document
}

/// Composed selection depth of exactly 500 split across two fragments.
fn depth_limit_document() -> String {
    format!(
        "query {{ ...a }} fragment a on Query {{ {}...b{} }} fragment b on Query {{ {}__typename{} }}",
        "... { ".repeat(248),
        " }".repeat(248),
        "... { ".repeat(249),
        " }".repeat(249)
    )
}

fn execute(document: String, introspection: bool) -> serde_json::Value {
    let schema =
        Schema::parse_and_validate("type Query { hello: String }", "schema.graphql").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let response = runtime.block_on(necrassrs::execute_with_options(
        &schema,
        &Request::new(document),
        &NoDispatch,
        &(),
        ExecutionOptions {
            introspection,
            ..ExecutionOptions::default()
        },
    ));
    assert!(
        !response.is_syntax_error(),
        "document must pass the parser: {}",
        serde_json::to_string(&response).unwrap()
    );
    serde_json::to_value(response).unwrap()
}

fn assert_single_recursion_error(response: serde_json::Value) {
    assert!(response.get("data").is_none(), "{response}");
    assert_eq!(
        response["errors"],
        serde_json::json!([{ "message": "too much recursion" }]),
        "{response}"
    );
}

/// Runs `case` in a child process on a fixed 2 MiB thread, the default Tokio worker stack size.
fn run_case(case: &str) {
    if let Ok(selected) = std::env::var(CHILD_ENV) {
        std::thread::Builder::new()
            .stack_size(STACK_BYTES)
            .spawn(move || run_child(&selected))
            .unwrap()
            .join()
            .unwrap();
        println!("{CHILD_MARKER}");
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([case, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, case)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains(CHILD_MARKER),
        "child for {case} failed with {:?}\nstdout:\n{stdout}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_child(case: &str) {
    match case {
        "deep_introspection_document_is_a_request_error" => {
            assert_single_recursion_error(execute(deep_of_type_document(), true))
        }
        "deep_document_is_rejected_with_introspection_disabled" => {
            assert_single_recursion_error(execute(deep_of_type_document(), false))
        }
        "document_at_depth_limit_executes" => {
            let response = execute(depth_limit_document(), true);
            assert_eq!(
                response,
                serde_json::json!({ "data": { "__typename": "Query" } }),
                "{response}"
            );
        }
        other => panic!("unknown case {other}"),
    }
}

#[test]
fn deep_introspection_document_is_a_request_error() {
    run_case("deep_introspection_document_is_a_request_error");
}

#[test]
fn deep_document_is_rejected_with_introspection_disabled() {
    run_case("deep_document_is_rejected_with_introspection_disabled");
}

#[test]
fn document_at_depth_limit_executes() {
    run_case("document_at_depth_limit_executes");
}
