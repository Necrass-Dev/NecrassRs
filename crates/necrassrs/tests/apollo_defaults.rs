//! Public dependency regressions: no NecrassRs request normalization.
use apollo_compiler::response::{JsonMap, serde_json_bytes::json};
use apollo_compiler::{ExecutableDocument, Schema, request::coerce_variable_values};

#[test]
fn variable_defaults_are_coerced_by_apollo() {
    // Reuse the three execution default regressions at the dependency boundary.
    for (sdl, query, name, expected) in [
        (
            "type Query { hello(names: [String!]): String }",
            "query($names: [String!] = \"Sheri\") { hello(names: $names) }",
            "names",
            json!(["Sheri"]),
        ),
        (
            "input GreetingInput { name: String! = \"Sheri\" } type Query { hello(input: GreetingInput): String }",
            "query($input: GreetingInput = {}) { hello(input: $input) }",
            "input",
            json!({"name": "Sheri"}),
        ),
        (
            "input GreetingInput { names: [String!] = \"Sheri\" } type Query { hello(input: GreetingInput): String }",
            "query($input: GreetingInput = {}) { hello(input: $input) }",
            "input",
            json!({"names": ["Sheri"]}),
        ),
    ] {
        let schema = Schema::parse_and_validate(sdl, "schema.graphql").unwrap();
        let document =
            ExecutableDocument::parse_and_validate(&schema, query, "query.graphql").unwrap();
        let operation = document.operations.get(None).unwrap();
        let values = coerce_variable_values(&schema, operation, &JsonMap::new()).unwrap();
        assert_eq!(values.get(name), Some(&expected));
    }
}

#[test]
fn supplied_objects_apply_nested_defaults_without_replacing_null_or_values() {
    let schema = Schema::parse_and_validate(
        "input Inner { names: [[String!]] = \"Sheri\" } \
         input Outer { inner: Inner = {} } type Query { hello(input: Outer): String }",
        "schema.graphql",
    )
    .unwrap();
    let document = ExecutableDocument::parse_and_validate(
        &schema,
        "query($input: Outer) { hello(input: $input) }",
        "query.graphql",
    )
    .unwrap();
    let operation = document.operations.get(None).unwrap();
    for (value, expected) in [
        (json!({}), json!({"inner": {"names": [["Sheri"]]}})),
        (json!({"inner": null}), json!({"inner": null})),
        (
            json!({"inner": {"names": "Other"}}),
            json!({"inner": {"names": [["Other"]]}}),
        ),
    ] {
        let mut variables = JsonMap::new();
        variables.insert("input", value);
        let values = coerce_variable_values(&schema, operation, &variables).unwrap();
        assert_eq!(values.get("input"), Some(&expected));
    }
    assert!(
        coerce_variable_values(&schema, operation, &JsonMap::new())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn graph_validation_distinguishes_defaults_from_type_recursion() {
    for input in [
        "input R { next: R }",
        "input R { next: R = {next: null} }",
        "input R { next: R = {next: {next: null}} }",
        "input A { bs: [B] = [] } input B { a: A = {} }",
        "input A { bs: [[B]] = [[null]] } input B { a: A = {} }",
        "input R { a: R = {a: null, b: null} b: R = {a: null, b: null} }",
    ] {
        Schema::parse_and_validate(
            format!("{input} type Query {{ hello: String }}"),
            "finite.graphql",
        )
        .unwrap();
    }
    for input in [
        "input R { next: R = {} }",
        "input A { bs: [B] = {} } input B { a: A = {} }",
        "input A { bs: [[B]] = [[{}]] } input B { a: A = {} }",
        "input R { a: R = {a: null} b: R = {b: null} }",
    ] {
        let errors = Schema::parse_and_validate(
            format!("{input} type Query {{ hello: String }}"),
            "cycle.graphql",
        )
        .unwrap_err();
        assert!(errors.errors.iter().any(|d| {
            let error = d.to_json();
            error.message.contains("defaults contain a cycle") && !error.locations.is_empty()
        }));
    }
    let schema = Schema::builder()
        .parse(
            "input A { b: B = {} } type Query { hello: String }",
            "a.graphql",
        )
        .parse(
            "input B { value: String } extend input B { a: A = {} }",
            "b.graphql",
        )
        .build()
        .unwrap();
    let errors = schema.validate().unwrap_err();
    let report = errors.to_string();
    assert!(report.contains("A.b") && report.contains("B.a"), "{report}");
    assert!(
        report.contains("a.graphql") && report.contains("b.graphql"),
        "{report}"
    );
}

#[test]
fn graph_validation_handles_shared_and_long_default_chains() {
    // Validation inspects dependencies without materializing the exponential value.
    for (depth, branching) in [(24, true), (1024, false)] {
        let mut sdl = "type Query { hello: String } input Leaf { value: String }".to_owned();
        for i in 0..depth {
            let next = if i + 1 == depth {
                "Leaf".to_owned()
            } else {
                format!("T{}", i + 1)
            };
            sdl.push_str(&format!(" input T{i} {{ a: {next} = {{}}"));
            if branching {
                sdl.push_str(&format!(" b: {next} = {{}}"));
            }
            sdl.push_str(" }");
        }
        Schema::parse_and_validate(sdl, "chain.graphql").unwrap();
    }
}
