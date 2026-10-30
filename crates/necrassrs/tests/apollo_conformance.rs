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
fn strings_decode_unicode_scalar_escapes() {
    for (literal, expected) in [
        (r#""\uD83D\uDE00""#, "😀"),
        (r#""\u{1F600}""#, "😀"),
        (r#""\u{0000000041}""#, "A"),
        (r#""\uDBFF\uDFFF""#, "\u{10ffff}"),
    ] {
        let document = apollo_compiler::ast::Document::parse(
            format!("query Q($s: String = {literal}) {{ value }}"),
            "request.graphql",
        )
        .unwrap();
        let apollo_compiler::ast::Definition::OperationDefinition(operation) =
            &document.definitions[0]
        else {
            panic!("expected operation")
        };
        assert_eq!(
            operation.variables[0]
                .default_value
                .as_ref()
                .unwrap()
                .as_str(),
            Some(expected)
        );
    }
    for literal in [
        r#""\uD800""#,
        r#""\uDC00""#,
        r#""\uD800\u0041""#,
        r#""\uD800\uD800""#,
        r#""\u{}""#,
        r#""\u{110000}""#,
        r#""\u{D800}""#,
        r#""\u{xyz}""#,
        r#""\u{FFFFFFFFFFFFFFFFFFFFFFFF}""#,
        r#""\u{1F600""#,
    ] {
        assert!(
            apollo_compiler::ast::Document::parse(
                format!("{{ value(s: {literal}) }}"),
                "request.graphql"
            )
            .is_err(),
            "{literal}"
        );
    }
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
fn schema_rejects_one_of_when_all_alternatives_recurse() {
    for fields in [
        "left: Choice right: Choice value: Int",
        "left: Choice right: [Choice!]",
    ] {
        Schema::parse_and_validate(
            format!(
                "input Choice @oneOf {{ {fields} }} type Query {{ inspect(choice: Choice): Int }}"
            ),
            "schema.graphql",
        )
        .expect("a scalar or list alternative must allow a finite input value");
    }

    let schema = Schema::parse(
        "input Choice @oneOf { left: Choice right: Choice } \
         type Query { inspect(choice: Choice): Int }",
        "schema.graphql",
    )
    .unwrap();
    let errors = schema
        .validate()
        .expect_err("every OneOf alternative recurses, so no finite input value exists");
    assert!(
        errors
            .errors
            .iter()
            .any(|error| !error.to_json().locations.is_empty())
    );
}

#[test]
fn schema_rejects_one_of_cycles_through_non_root_ancestors() {
    for escape in ["value: Int", "values: [B!]", "optional: End"] {
        Schema::parse_and_validate(
            format!("input A @oneOf {{ a: A b: B }} input B @oneOf {{ a: A b: B {escape} }} input End {{ next: A }} type Query {{ value(input: A): Int }}"),
            "schema.graphql",
        ).expect("a finite alternative in a mutually recursive component makes it inhabitable");
    }
    let errors = Schema::parse_and_validate(
        "input A @oneOf { a: A b: B } input B @oneOf { a: A b: B } type Query { value(input: A): Int }",
        "schema.graphql",
    ).expect_err("revisiting any ancestor is a cycle, even when it is not the initial type");
    assert!(
        errors
            .errors
            .iter()
            .any(|error| !error.to_json().locations.is_empty())
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
fn schema_accepts_directive_extensions() {
    let schema = Schema::builder()
        .parse(
            "directive @old on FIELD type Query { value: Int }",
            "base.graphql",
        )
        .parse(
            "extend directive @old @deprecated(reason: \"Retired\")",
            "extension.graphql",
        )
        .build()
        .expect("directive extensions must be merged with their definitions")
        .validate()
        .unwrap();
    assert!(schema.directive_definitions.contains_key("old"));
    let serialized = schema.to_string();
    assert!(
        serialized.contains(r#"@deprecated(reason: "Retired")"#),
        "{serialized}"
    );
    Schema::parse_and_validate(serialized, "roundtrip.graphql").unwrap();
}

#[test]
fn schema_accepts_directive_definition_location() {
    let schema = Schema::parse_and_validate(
        "directive @metadata on DIRECTIVE_DEFINITION type Query { value: Int }",
        "schema.graphql",
    )
    .expect("DIRECTIVE_DEFINITION is a valid directive location");
    let response = introspect(&schema, "{ __schema { directives { name locations } } }");
    let metadata = response["data"]["__schema"]["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|directive| directive["name"] == "metadata")
        .unwrap();
    assert_eq!(metadata["locations"], json!(["DIRECTIVE_DEFINITION"]));
}

#[test]
fn introspection_reports_directive_deprecation_metadata() {
    let schema = Schema::parse_and_validate(
        "directive @current on FIELD type Query { value: Int }",
        "schema.graphql",
    )
    .unwrap();
    let response = introspect(
        &schema,
        "{ __schema { directives { name isDeprecated deprecationReason } } }",
    );
    let current = response["data"]["__schema"]["directives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|directive| directive["name"] == "current")
        .unwrap();
    assert_eq!(
        current,
        &json!({"name": "current", "isDeprecated": false, "deprecationReason": null})
    );
}

#[test]
fn introspection_filters_deprecated_directives() {
    let schema = Schema::parse_and_validate("type Query { value: Int }", "schema.graphql").unwrap();
    introspect(
        &schema,
        "{ __schema { directives(includeDeprecated: true) { name } } }",
    );
    let schema = Schema::parse_and_validate(
        "directive @old @deprecated(reason: \"Use @current\") on FIELD directive @current on FIELD type Query { value: Int }", "schema.graphql",
    ).unwrap();
    for (argument, expected_old) in [
        ("", false),
        ("(includeDeprecated: false)", false),
        ("(includeDeprecated: true)", true),
    ] {
        let query = format!(
            "{{ __schema {{ directives{argument} {{ name isDeprecated deprecationReason }} }} }}"
        );
        let response = introspect(&schema, &query);
        let directives = response["data"]["__schema"]["directives"]
            .as_array()
            .unwrap();
        let old = directives
            .iter()
            .find(|directive| directive["name"] == "old");
        assert_eq!(old.is_some(), expected_old);
        if let Some(old) = old {
            assert_eq!(
                old,
                &json!({"name": "old", "isDeprecated": true, "deprecationReason": "Use @current"})
            );
        }
        assert!(
            directives
                .iter()
                .any(|directive| directive["name"] == "current")
        );
    }
}

#[test]
fn schema_rejects_incompatible_field_argument_default() {
    Schema::parse_and_validate(
        "type Query { inspect(value: Int = 1): Int }",
        "schema.graphql",
    )
    .unwrap();
    let schema = Schema::parse(
        "type Query { inspect(value: Int = \"bad\"): Int }",
        "schema.graphql",
    )
    .unwrap();
    schema
        .validate()
        .expect_err("a field argument default must be coercible to its declared type");
}

#[test]
fn schema_rejects_incompatible_input_field_default() {
    Schema::parse_and_validate(
        "input Input { value: Int = 1 } type Query { inspect(input: Input): Int }",
        "schema.graphql",
    )
    .unwrap();
    let schema = Schema::parse(
        "input Input { value: Int = \"bad\" } type Query { inspect(input: Input): Int }",
        "schema.graphql",
    )
    .unwrap();
    schema
        .validate()
        .expect_err("an input field default must be coercible to its declared type");
}

#[test]
fn schema_rejects_incompatible_directive_argument_default() {
    Schema::parse_and_validate(
        "directive @example(value: Int = 1) on FIELD type Query { value: Int }",
        "schema.graphql",
    )
    .unwrap();
    let schema = Schema::parse(
        "directive @example(value: Int = \"bad\") on FIELD type Query { value: Int }",
        "schema.graphql",
    )
    .unwrap();
    schema
        .validate()
        .expect_err("a directive argument default must be coercible to its declared type");
}

#[test]
fn schema_rejects_empty_one_of_argument_default() {
    Schema::parse_and_validate("input Choice @oneOf { value: Int other: Int } type Query { inspect(choice: Choice = {value: 1}): Int }", "schema.graphql").unwrap();
    let schema = Schema::parse("input Choice @oneOf { value: Int other: Int } type Query { inspect(choice: Choice = {}): Int }", "schema.graphql").unwrap();
    schema
        .validate()
        .expect_err("a OneOf argument default must select exactly one non-null member");
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
