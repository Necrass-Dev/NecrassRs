//! Issue #28 regressions against the baseline pinned in archives/specs.md.
use apollo_compiler::{
    ExecutableDocument, Schema, introspection, request::coerce_variable_values, response::JsonMap,
    validation::Valid,
};
use serde_json::{Value, json};

fn introspect(schema: &Valid<Schema>, query: &str) -> Value {
    let document = ExecutableDocument::parse_and_validate(schema, query, "request.graphql")
        .expect("the introspection document must be valid");
    let operation = document.operations.get(None).unwrap();
    let variables = coerce_variable_values(schema, operation, &JsonMap::new()).unwrap();
    let response = introspection::partial_execute(
        schema,
        &schema.implementers_map(),
        &document,
        operation,
        &variables,
    )
    .unwrap();
    let response = serde_json::to_value(response).unwrap();
    assert!(response.get("errors").is_none(), "{response}");
    response
}

#[test]
fn introspection_rejects_null_include_deprecated() {
    let schema = Schema::parse_and_validate("type Query { value: Int }", "schema.graphql").unwrap();
    for argument in [
        "",
        "(includeDeprecated: false)",
        "(includeDeprecated: true)",
    ] {
        let query = format!(r#"{{ __type(name: "Query") {{ fields{argument} {{ name }} }} }}"#);
        ExecutableDocument::parse_and_validate(&schema, query, "request.graphql").unwrap();
    }
    let document = apollo_compiler::ast::Document::parse(
        r#"{ __type(name: "Query") { fields(includeDeprecated: null) { name } } }"#,
        "request.graphql",
    )
    .unwrap();
    let errors = document
        .to_executable_validate(&schema)
        .expect_err("includeDeprecated has type Boolean! and must reject explicit null");
    assert!(
        errors
            .errors
            .iter()
            .any(|error| !error.to_json().locations.is_empty())
    );
}

#[test]
fn introspection_reports_one_of_flags() {
    let schema = Schema::parse_and_validate(
        "input Choice @oneOf { value: Int } input Ordinary { value: Int } type Query { value: Int }",
        "schema.graphql",
    ).unwrap();
    let response = introspect(
        &schema,
        r#"{
        choice: __type(name: "Choice") { isOneOf }
        ordinary: __type(name: "Ordinary") { isOneOf }
        object: __type(name: "Query") { isOneOf }
        scalar: __type(name: "Int") { isOneOf }
    }"#,
    );
    assert_eq!(
        response,
        json!({"data": {
            "choice": {"isOneOf": true},
            "ordinary": {"isOneOf": false},
            "object": {"isOneOf": null},
            "scalar": {"isOneOf": null}
        }})
    );
}
