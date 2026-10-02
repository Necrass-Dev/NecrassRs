//! OneOf registration and variable coercion at the public Apollo boundary.
use apollo_compiler::response::{JsonMap, serde_json_bytes::json};
use apollo_compiler::{ExecutableDocument, Schema, request::coerce_variable_values};

const SCHEMA: &str = r#"
    directive @oneOf on INPUT_OBJECT
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
        [apollo_compiler::ast::DirectiveLocation::InputObject],
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
