//! OneOf registration and variable coercion at the public Lead boundary.
use lead_compiler::ast;
use lead_compiler::response::{serde_json_bytes::json, JsonMap};
use lead_compiler::{request::coerce_variable_values, ExecutableDocument, Schema};

const SCHEMA: &str = r#"
    input Nested { name: String! = "Sheri" }
    input Choice @oneOf { number: Int items: [Int] nested: Nested }
    input Wrapper { choice: Choice choices: [Choice] }
    input Ordinary { number: Int other: Int }
    type Query { inspect(choice: Choice, wrapper: Wrapper, ordinary: Ordinary): String }
"#;

#[test]
fn one_of_directive_is_available_without_an_sdl_declaration() {
    let schema = Schema::parse_and_validate(
        "input Choice @oneOf { number: Int } type Query { inspect(choice: Choice): String }",
        "schema.graphql",
    )
    .expect("OneOf must be available as a built-in directive");
    let directive = &schema.directive_definitions["oneOf"];
    assert!(!directive.repeatable);
    assert!(directive.arguments.is_empty());
    assert_eq!(
        directive.locations,
        [lead_compiler::ast::DirectiveLocation::InputObject],
    );
}

#[test]
fn one_of_variable_values_preserve_valid_selections_and_nested_coercion() {
    let schema = Schema::parse_and_validate(SCHEMA, "schema.graphql").unwrap();
    for (ty, argument, supplied, expected) in [
        ("Choice", "choice", json!(null), json!(null)),
        (
            "Choice",
            "choice",
            json!({"number": 1}),
            json!({"number": 1}),
        ),
        (
            "Choice",
            "choice",
            json!({"items": 2}),
            json!({"items": [2]}),
        ),
        (
            "Choice",
            "choice",
            json!({"items": [1, null]}),
            json!({"items": [1, null]}),
        ),
        (
            "Choice",
            "choice",
            json!({"nested": {}}),
            json!({"nested": {"name": "Sheri"}}),
        ),
        (
            "Wrapper",
            "wrapper",
            json!({"choice": {"number": 1}, "choices": [{"items": 2}, null]}),
            json!({"choice": {"number": 1}, "choices": [{"items": [2]}, null]}),
        ),
        (
            "Ordinary",
            "ordinary",
            json!({"number": 1, "other": null}),
            json!({"number": 1, "other": null}),
        ),
    ] {
        let query = format!("query($value: {ty}) {{ inspect({argument}: $value) }}");
        let document =
            ExecutableDocument::parse_and_validate(&schema, query, "query.graphql").unwrap();
        let supplied: JsonMap = [("value".into(), supplied)].into_iter().collect();
        let values =
            coerce_variable_values(&schema, document.operations.get(None).unwrap(), &supplied)
                .unwrap();
        assert_eq!(values.get("value"), Some(&expected), "{ty}");
    }
}

#[test]
fn one_of_variable_values_reject_invalid_selections_before_execution() {
    let schema = Schema::parse_and_validate(SCHEMA, "schema.graphql").unwrap();
    let mut accepted = Vec::new();
    for (ty, argument, supplied) in [
        ("Choice", "choice", json!({})),
        ("Choice", "choice", json!({"number": null})),
        ("Choice", "choice", json!({"number": 1, "items": [2]})),
        ("Choice", "choice", json!({"number": 1, "items": null})),
        ("Choice", "choice", json!({"unknown": 1})),
        ("Wrapper", "wrapper", json!({"choice": {}})),
        (
            "Wrapper",
            "wrapper",
            json!({"choices": [{"number": 1}, {"number": null}]}),
        ),
    ] {
        let query = format!("query($value: {ty}) {{ inspect({argument}: $value) }}");
        let document =
            ExecutableDocument::parse_and_validate(&schema, query, "query.graphql").unwrap();
        let values: JsonMap = [("value".into(), supplied.clone())].into_iter().collect();
        match coerce_variable_values(&schema, document.operations.get(None).unwrap(), &values) {
            Ok(_) => accepted.push(supplied),
            Err(error) => assert!(error.to_graphql_error(&document.sources).path.is_empty()),
        }
    }
    assert!(
        accepted.is_empty(),
        "invalid OneOf variable values accepted: {accepted:?}"
    );
}

#[test]
fn one_of_variable_defaults_use_the_same_recursive_coercion() {
    let schema = Schema::parse_and_validate(SCHEMA, "schema.graphql").unwrap();
    for (ty, argument, default, expected) in [
        ("Choice", "choice", "{items: 2}", json!({"items": [2]})),
        (
            "Choice",
            "choice",
            "{nested: {}}",
            json!({"nested": {"name": "Sheri"}}),
        ),
        (
            "Wrapper",
            "wrapper",
            "{choice: {items: 2}}",
            json!({"choice": {"items": [2]}}),
        ),
    ] {
        let query = format!("query($value: {ty} = {default}) {{ inspect({argument}: $value) }}");
        let document =
            ExecutableDocument::parse_and_validate(&schema, query, "query.graphql").unwrap();
        let values = coerce_variable_values(
            &schema,
            document.operations.get(None).unwrap(),
            &JsonMap::new(),
        )
        .unwrap();
        assert_eq!(values.get("value"), Some(&expected), "{default}");
    }
}

#[test]
fn one_of_schema_accepts_nullable_fields_without_defaults() {
    Schema::parse_and_validate(
        r#"
            input Nested { name: String! }
            input Choice @oneOf {
                number: Int
                nested: Nested
                items: [Int!]
            }
            input Ordinary { required: Int! defaulted: Int = 1 }
            type Query { inspect(choice: Choice, ordinary: Ordinary): String }
        "#,
        "schema.graphql",
    )
    .expect("nullable OneOf fields and ordinary input fields must remain valid");
}

#[test]
fn one_of_schema_rejects_non_null_fields() {
    for field_type in ["Int!", "[Int]!", "[Int!]!"] {
        let source = format!(
            "input Choice @oneOf {{ selected: {field_type} }} \
             type Query {{ inspect(choice: Choice): String }}"
        );
        let errors = Schema::parse_and_validate(source, "schema.graphql")
            .err()
            .unwrap_or_else(|| panic!("OneOf field type {field_type} must be rejected"));
        assert!(!errors.errors.is_empty(), "{field_type}");
    }
}

#[test]
fn one_of_schema_rejects_field_defaults_including_null() {
    for default in ["null", "0", "1"] {
        let source = format!(
            "input Choice @oneOf {{ selected: Int = {default} }} \
             type Query {{ inspect(choice: Choice): String }}"
        );
        let errors = Schema::parse_and_validate(source, "schema.graphql")
            .err()
            .unwrap_or_else(|| panic!("OneOf field default {default} must be rejected"));
        assert!(!errors.errors.is_empty(), "default {default}");
    }
}

#[test]
fn one_of_document_accepts_valid_selections() {
    let schema = one_of_document_schema();
    for source in [
        "{ inspect(choice: {number: 1}) }",
        "{ inspect(choice: {items: [1, null]}) }",
        "{ inspect(choice: null) }",
        "{ inspect }",
        "{ inspect(wrapper: {choice: {number: 1}}) }",
        "{ inspect(choices: [{number: 1}, {items: [null]}]) }",
        "query($number: Int!) { inspect(choice: {number: $number}) }",
        "query($items: [Int]!) { inspect(choice: {items: $items}) }",
        "query($choice: Choice) { inspect(choice: $choice) }",
        "query($number: Int = 1) { inspect(ordinary: {number: $number, other: null}) }",
    ] {
        let document = ast::Document::parse(source, "request.graphql").unwrap();
        document
            .to_executable_validate(&schema)
            .unwrap_or_else(|errors| panic!("valid OneOf document rejected: {source}\n{errors}"));
    }
}

#[test]
fn one_of_document_rejects_invalid_selections() {
    let schema = one_of_document_schema();
    let mut accepted = Vec::new();
    for source in [
        "{ inspect(choice: {}) }",
        "{ inspect(choice: {number: 1, items: [2]}) }",
        "{ inspect(choice: {number: 1, items: null}) }",
        "{ inspect(choice: {number: null}) }",
        "{ inspect(choice: {unknown: 1}) }",
        "{ inspect(wrapper: {choice: {}}) }",
        "{ inspect(choices: [{number: 1}, {}]) }",
        "query($number: Int) { inspect(choice: {number: $number}) }",
        "query($number: Int = 1) { inspect(choice: {number: $number}) }",
        "query($items: [Int]) { inspect(choice: {items: $items}) }",
        "query($items: [Int]) { inspect(choice: {number: 1, items: $items}) }",
        "query($number: Int!) { inspect(choice: {items: $number}) }",
    ] {
        let document = ast::Document::parse(source, "request.graphql").unwrap();
        match document.to_executable_validate(&schema) {
            Ok(_) => accepted.push(source),
            Err(errors) => assert!(
                errors
                    .errors
                    .iter()
                    .any(|error| !error.to_json().locations.is_empty()),
                "OneOf document errors must preserve source locations: {source}",
            ),
        }
    }
    assert!(
        accepted.is_empty(),
        "invalid OneOf documents accepted: {accepted:#?}"
    );
}

fn one_of_document_schema() -> lead_compiler::validation::Valid<Schema> {
    Schema::parse_and_validate(
        r#"
            input Choice @oneOf { number: Int items: [Int] }
            input Wrapper { choice: Choice }
            input Ordinary { number: Int other: Int }
            type Query {
                inspect(choice: Choice, wrapper: Wrapper, choices: [Choice], ordinary: Ordinary): String
            }
        "#,
        "schema.graphql",
    )
    .unwrap()
}
