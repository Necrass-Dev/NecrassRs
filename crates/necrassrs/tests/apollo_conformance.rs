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
fn schema_rejects_null_deprecation_reason() {
    Schema::parse_and_validate("type Query { value: Int @deprecated }", "schema.graphql").unwrap();
    let schema = Schema::parse(
        "type Query { value: Int @deprecated(reason: null) }",
        "schema.graphql",
    )
    .unwrap();
    schema
        .validate()
        .expect_err("@deprecated reason has type String! and must reject null");
}

#[test]
fn schema_rejects_deprecated_required_field_argument() {
    for argument in ["value: Int @deprecated", "value: Int! = 1 @deprecated"] {
        Schema::parse_and_validate(
            format!("type Query {{ inspect({argument}): Int }}"),
            "schema.graphql",
        )
        .unwrap();
    }
    let schema = Schema::parse(
        "type Query { inspect(value: Int! @deprecated): Int }",
        "schema.graphql",
    )
    .unwrap();
    schema
        .validate()
        .expect_err("a required field argument cannot be deprecated");
}

#[test]
fn schema_rejects_deprecated_required_input_field() {
    for field in ["value: Int @deprecated", "value: Int! = 1 @deprecated"] {
        Schema::parse_and_validate(
            format!("input Input {{ {field} }} type Query {{ inspect(input: Input): Int }}"),
            "schema.graphql",
        )
        .unwrap();
    }
    let schema = Schema::parse(
        "input Input { value: Int! @deprecated } type Query { inspect(input: Input): Int }",
        "schema.graphql",
    )
    .unwrap();
    schema
        .validate()
        .expect_err("a required input field cannot be deprecated");
}

#[test]
fn schema_rejects_deprecated_required_directive_argument() {
    for argument in ["value: Int @deprecated", "value: Int! = 1 @deprecated"] {
        Schema::parse_and_validate(
            format!("directive @example({argument}) on FIELD type Query {{ value: Int }}"),
            "schema.graphql",
        )
        .unwrap();
    }
    let schema = Schema::parse(
        "directive @example(value: Int! @deprecated) on FIELD type Query { value: Int }",
        "schema.graphql",
    )
    .unwrap();
    schema
        .validate()
        .expect_err("a required directive argument cannot be deprecated");
}

#[test]
fn document_accepts_operation_descriptions() {
    let schema = Schema::parse_and_validate("type Query { value: Int }", "schema.graphql").unwrap();
    for description in [
        r#""Operation description""#,
        r#""""Operation description""""#,
    ] {
        let query = format!("{description} query Named {{ __typename }}");
        assert_eq!(
            introspect(&schema, &query),
            json!({"data": {"__typename": "Query"}})
        );
    }
    assert!(
        apollo_compiler::ast::Document::parse(r#""Description" { value }"#, "request.graphql")
            .is_err()
    );
}

#[test]
fn document_accepts_fragment_descriptions() {
    let schema = Schema::parse_and_validate("type Query { value: Int }", "schema.graphql").unwrap();
    for description in [r#""Fragment description""#, r#""""Fragment description""""#] {
        let query = format!(
            "query Named {{ ...Fields }} {description} fragment Fields on Query {{ __typename }}"
        );
        assert_eq!(
            introspect(&schema, &query),
            json!({"data": {"__typename": "Query"}})
        );
    }
}

#[test]
fn document_accepts_empty_root_selection() {
    let schema = Schema::parse_and_validate("type Query { value: Int }", "schema.graphql").unwrap();
    for query in ["{}", "query Named {}"] {
        assert_eq!(introspect(&schema, query), json!({"data": {}}));
    }
}

#[test]
fn document_distinguishes_empty_and_absent_composite_selections() {
    let schema = Schema::parse_and_validate(
        "type Query { object: Object value: Int } type Object { value: Int }",
        "schema.graphql",
    )
    .unwrap();
    ExecutableDocument::parse_and_validate(&schema, "{ object {} }", "request.graphql")
        .expect("an explicit empty composite selection is valid");
    assert_eq!(
        introspect(&schema, r#"{ __type(name: "Query") {} }"#),
        json!({"data": {"__type": {}}})
    );
    for query in ["{ object }", "{ value {} }"] {
        let document = apollo_compiler::ast::Document::parse(query, "request.graphql").unwrap();
        document
            .to_executable_validate(&schema)
            .expect_err("composites require a selection set and leaves forbid one");
    }
}

#[test]
fn schema_rejects_uninhabitable_one_of_self_cycle() {
    for fields in ["next: Choice value: Int", "next: [Choice!]"] {
        Schema::parse_and_validate(
            format!(
                "input Choice @oneOf {{ {fields} }} type Query {{ inspect(choice: Choice): Int }}"
            ),
            "schema.graphql",
        )
        .unwrap();
    }
    let schema = Schema::parse(
        "input Choice @oneOf { next: Choice } type Query { inspect(choice: Choice): Int }",
        "schema.graphql",
    )
    .unwrap();
    schema
        .validate()
        .expect_err("a OneOf cycle without a finite alternative is invalid even without defaults");
}

#[test]
fn schema_rejects_uninhabitable_one_of_mutual_cycle() {
    Schema::parse_and_validate("input Choice @oneOf { next: Input } input Input { choice: Choice } type Query { inspect(choice: Choice): Int }", "schema.graphql").unwrap();
    let schema = Schema::parse("input Choice @oneOf { next: Input } input Input { choice: Choice! } type Query { inspect(choice: Choice): Int }", "schema.graphql").unwrap();
    schema.validate().expect_err(
        "a mandatory return edge through an ordinary input cannot terminate a OneOf cycle",
    );
}

#[test]
fn schema_accepts_directives_on_directive_definitions() {
    let schema = Schema::parse_and_validate(
        "directive @old @deprecated(reason: \"Use @current\") on FIELD type Query { value: Int }",
        "schema.graphql",
    )
    .expect("directive definitions may be annotated with @deprecated");
    let serialized = schema.to_string();
    assert!(
        serialized.contains(r#"@deprecated(reason: "Use @current")"#),
        "{serialized}"
    );
    let reparsed = Schema::parse_and_validate(&serialized, "roundtrip.graphql").unwrap();
    assert_eq!(reparsed.directive_definitions["old"].name.as_str(), "old");
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
