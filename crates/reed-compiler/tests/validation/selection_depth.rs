//! Composed selection depth is bounded across fragment spreads.
//!
//! The selection depth of a definition counts its own selection set as 1 and adds one level for
//! each non-empty selection set entered through a field, an inline fragment, or a fragment spread.
//! Depth above 500 is rejected with a single recursion diagnostic before recursive validation runs.

use reed_compiler::ExecutableDocument;
use reed_compiler::Schema;
use std::process::Command;

const STACK_BYTES: usize = 2 * 1024 * 1024;
const CHILD_ENV: &str = "REED_SELECTION_DEPTH_CHILD";
const CHILD_MARKER: &str = "SELECTION_DEPTH_CHILD_RAN";

fn schema() -> reed_compiler::validation::Valid<Schema> {
    Schema::parse_and_validate("type Query { hello: String }", "schema.graphql").unwrap()
}

/// Opens `levels` anonymous inline fragments around `inner`.
fn inline_nest(levels: usize, inner: &str) -> String {
    format!("{}{inner}{}", "... { ".repeat(levels), " }".repeat(levels))
}

/// `fragments` chained fragments; each nests `levels` inline fragments, then spreads the next.
fn inline_chain(fragments: usize, levels: usize) -> String {
    let mut document = String::from("query { ...f1 }");
    for i in 1..=fragments {
        let inner = if i < fragments {
            format!("...f{}", i + 1)
        } else {
            "__typename".to_string()
        };
        document.push_str(&format!(
            "\nfragment f{i} on Query {{ {} }}",
            inline_nest(levels, &inner)
        ));
    }
    document
}

/// `fragments` chained introspection fragments; each nests `levels` `ofType` fields.
fn of_type_chain(fragments: usize, levels: usize) -> String {
    let mut document = String::from("query { __schema { types { ...f1 } } }");
    for i in 1..=fragments {
        let inner = if i < fragments {
            format!("...f{}", i + 1)
        } else {
            "name".to_string()
        };
        document.push_str(&format!(
            "\nfragment f{i} on __Type {{ {}{inner}{} }}",
            "ofType { ".repeat(levels),
            " }".repeat(levels)
        ));
    }
    document
}

fn errors(document: String) -> Vec<String> {
    match ExecutableDocument::parse_and_validate(&schema(), document, "query.graphql") {
        Ok(_) => Vec::new(),
        Err(error) => error
            .errors
            .iter()
            .map(|diagnostic| diagnostic.to_json().message)
            .collect(),
    }
}

fn assert_valid(document: String) {
    assert_eq!(errors(document), Vec::<String>::new());
}

fn assert_single_recursion_error(document: String) {
    assert_eq!(errors(document), ["too much recursion"]);
}

/// Each case runs in a child process on a fixed 2 MiB thread, so a stack overflow aborts only the
/// child and an inherited RUST_MIN_STACK cannot hide the regression.
fn run_case(case: &str) {
    if let Ok(selected) = std::env::var(CHILD_ENV) {
        let selected = selected.clone();
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
        .args([
            &format!("validation::selection_depth::{case}"),
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
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
        "deep_of_type_chain_is_rejected_without_overflow" => {
            assert_single_recursion_error(of_type_chain(99, 499))
        }
        "deep_inline_fragment_chain_is_rejected_without_overflow" => {
            assert_single_recursion_error(inline_chain(99, 499))
        }
        "deep_mixed_chain_is_rejected_without_overflow" => {
            let mut document = String::from("query { __schema { types { ...f1 } } }");
            for i in 1..=99 {
                let inner = if i < 99 {
                    format!("...f{}", i + 1)
                } else {
                    "name".to_string()
                };
                document.push_str(&format!(
                    "\nfragment f{i} on __Type {{ {}{inner}{} }}",
                    "ofType { ... { ".repeat(120),
                    " } }".repeat(120)
                ));
            }
            assert_single_recursion_error(document)
        }
        "deep_fragment_cycle_is_rejected_without_overflow" => {
            let document = (0..3)
                .map(|i| {
                    format!(
                        "fragment f{i} on Query {{ {} }}",
                        inline_nest(400, &format!("...f{}", (i + 1) % 3))
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert_single_recursion_error(format!("query {{ ...f0 }}\n{document}"))
        }
        other => panic!("unknown case {other}"),
    }
}

#[test]
fn deep_of_type_chain_is_rejected_without_overflow() {
    run_case("deep_of_type_chain_is_rejected_without_overflow");
}

#[test]
fn deep_inline_fragment_chain_is_rejected_without_overflow() {
    run_case("deep_inline_fragment_chain_is_rejected_without_overflow");
}

#[test]
fn deep_mixed_chain_is_rejected_without_overflow() {
    run_case("deep_mixed_chain_is_rejected_without_overflow");
}

#[test]
fn deep_fragment_cycle_is_rejected_without_overflow() {
    run_case("deep_fragment_cycle_is_rejected_without_overflow");
}

#[test]
fn operation_selection_depth_at_limit_is_accepted() {
    // Root selection set (1) plus 499 inline fragments.
    assert_valid(format!("query {{ {} }}", inline_nest(499, "__typename")));
}

#[test]
fn fragment_selection_depth_at_limit_is_accepted() {
    // Root (1) + spread (1) + 498 inline fragments.
    assert_valid(format!(
        "query {{ ...f }} fragment f on Query {{ {} }}",
        inline_nest(498, "__typename")
    ));
}

#[test]
fn fragment_selection_depth_over_limit_is_rejected() {
    assert_single_recursion_error(format!(
        "query {{ ...f }} fragment f on Query {{ {} }}",
        inline_nest(499, "__typename")
    ));
}

#[test]
fn split_selection_depth_at_limit_is_accepted() {
    // Root (1) + spread a (1) + 248 + spread b (1) + 249 = 500.
    assert_valid(format!(
        "query {{ ...a }} fragment a on Query {{ {} }} fragment b on Query {{ {} }}",
        inline_nest(248, "...b"),
        inline_nest(249, "__typename")
    ));
}

#[test]
fn split_selection_depth_over_limit_is_rejected() {
    assert_single_recursion_error(format!(
        "query {{ ...a }} fragment a on Query {{ {} }} fragment b on Query {{ {} }}",
        inline_nest(248, "...b"),
        inline_nest(250, "__typename")
    ));
}

#[test]
fn unused_fragment_over_limit_is_rejected() {
    // Fragment a composes 1 + 248 + spread (1) + 251 = 501 without being used by an operation.
    assert_single_recursion_error(format!(
        "query {{ __typename }} fragment a on Query {{ {} }} fragment b on Query {{ {} }}",
        inline_nest(248, "...b"),
        inline_nest(251, "__typename")
    ));
}

#[test]
fn shallow_fragment_cycle_keeps_cycle_diagnostic() {
    assert_eq!(
        errors("query { ...a } fragment a on Query { ...b } fragment b on Query { ...a }".into()),
        ["`a` fragment cannot reference itself"]
    );
}

#[test]
fn moderate_fragment_hop_chain_reports_nesting_diagnostic() {
    // 150 hops stay within the depth limit but exceed the fragment hop limit of 100.
    let errors = errors(inline_chain(150, 1));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("contains too much nesting"),
        "{errors:?}"
    );
}
