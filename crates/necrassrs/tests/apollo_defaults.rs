//! Public dependency regressions: no NecrassRs request normalization.
use apollo_compiler::response::{JsonMap, serde_json_bytes::json};
use apollo_compiler::{ExecutableDocument, Schema, request::coerce_variable_values};

#[test]
fn id_defaults_preserve_integer_text_before_json_number_conversion() {
    let schema = Schema::parse_and_validate(
        "input Holder { id: ID = 18446744073709551617 ids: [ID] = -18446744073709551617 } type Query { inspect(id: ID, holder: Holder): String }",
        "schema.graphql",
    ).unwrap();
    for id in [
        "18446744073709551617".to_owned(),
        "-18446744073709551617".to_owned(),
        "9".repeat(400),
    ] {
        let query = format!(
            "query($id: ID = {id}, $holder: Holder = {{id: {id}, ids: {id}}}) {{inspect(id: $id, holder: $holder)}}"
        );
        let document =
            ExecutableDocument::parse_and_validate(&schema, query, "query.graphql").unwrap();
        let coerced = coerce_variable_values(
            &schema,
            document.operations.get(None).unwrap(),
            &JsonMap::new(),
        )
        .unwrap();
        assert_eq!(coerced["id"], json!(id));
        assert_eq!(coerced["holder"], json!({"id": id, "ids": [id]}));
    }
    let document = ExecutableDocument::parse_and_validate(
        &schema,
        "query($holder: Holder!) {inspect(holder: $holder)}",
        "query.graphql",
    )
    .unwrap();
    let supplied = [("holder".into(), json!({}))].into_iter().collect();
    let coerced =
        coerce_variable_values(&schema, document.operations.get(None).unwrap(), &supplied).unwrap();
    assert_eq!(
        coerced["holder"],
        json!({"id": "18446744073709551617", "ids": ["-18446744073709551617"]})
    );
}

#[test]
fn json_variable_numbers_are_normalized_without_changing_literal_kinds() {
    let schema = Schema::parse_and_validate(
        "input Numbers { integers: [Int!] ids: [ID!] } type Query { inspect(value: Numbers): String }",
        "schema.graphql",
    ).unwrap();
    let document = ExecutableDocument::parse_and_validate(
        &schema,
        "query($value: Numbers) { inspect(value: $value) }",
        "query.graphql",
    )
    .unwrap();
    let operation = document.operations.get(None).unwrap();
    let supplied: JsonMap = [(
        "value".into(),
        json!({
            "integers": [-2147483648.0, 1.0, 2147483647.0],
            "ids": [42.0, -42, "001", u64::MAX]
        }),
    )]
    .into_iter()
    .collect();
    let coerced = coerce_variable_values(&schema, operation, &supplied).unwrap();
    assert_eq!(
        coerced["value"],
        json!({
            "integers": [-2147483648i64, 1, 2147483647],
            "ids": ["42", -42, "001", u64::MAX]
        })
    );
    for invalid in [
        json!({"integers": [1.5]}),
        json!({"integers": [2147483648.0]}),
        json!({"integers": [-2147483649.0]}),
        json!({"ids": [1.5]}),
    ] {
        let supplied = [("value".into(), invalid)].into_iter().collect();
        assert!(coerce_variable_values(&schema, operation, &supplied).is_err());
    }
    for query in [
        "{ inspect(value: {integers: [1.0]}) }",
        "{ inspect(value: {ids: [42.0]}) }",
    ] {
        assert!(ExecutableDocument::parse_and_validate(&schema, query, "query.graphql").is_err());
    }
}

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
fn numeric_literal_defaults_preserve_validated_values() {
    let mut failures = Vec::new();
    for (scalar, literal, value) in [
        ("Float", "9007199254740991", json!(9_007_199_254_740_991u64)),
        ("ID", "9223372036854775808", json!("9223372036854775808")),
    ] {
        for (ty, expected) in [
            (scalar.to_owned(), value.clone()),
            (format!("[{scalar}]"), json!([value.clone()])),
            (format!("[[{scalar}]]"), json!([[value]])),
        ] {
            for (case, sdl, query, variables, expected) in [
                (
                    "variable default",
                    format!("type Query {{ hello(value: {ty}): String }}"),
                    format!("query($value: {ty} = {literal}) {{ hello(value: $value) }}"),
                    JsonMap::new(),
                    expected.clone(),
                ),
                (
                    "field default in supplied object",
                    format!(
                        "input I {{ value: {ty} = {literal} }} type Query {{ hello(value: I): String }}"
                    ),
                    "query($value: I) { hello(value: $value) }".to_owned(),
                    [("value".into(), json!({}))].into_iter().collect(),
                    json!({"value": expected.clone()}),
                ),
                (
                    "field default in variable default",
                    format!(
                        "input I {{ value: {ty} = {literal} }} type Query {{ hello(value: I): String }}"
                    ),
                    "query($value: I = {}) { hello(value: $value) }".to_owned(),
                    JsonMap::new(),
                    json!({"value": expected.clone()}),
                ),
            ] {
                let schema = Schema::parse_and_validate(sdl, "schema.graphql").unwrap();
                let document =
                    ExecutableDocument::parse_and_validate(&schema, query, "query.graphql")
                        .unwrap();
                let result = coerce_variable_values(
                    &schema,
                    document.operations.get(None).unwrap(),
                    &variables,
                );
                match result {
                    Ok(values) => assert_eq!(values.get("value"), Some(&expected), "{case}: {ty}"),
                    Err(error) => failures.push(format!("{case}: {ty}: {error:?}")),
                }
                // Supplied values must still take the untrusted JSON coercion path.
                let mut supplied = JsonMap::new();
                supplied.insert(
                    "value",
                    if expected.is_object() {
                        json!({"value": true})
                    } else {
                        json!(true)
                    },
                );
                assert!(
                    coerce_variable_values(
                        &schema,
                        document.operations.get(None).unwrap(),
                        &supplied,
                    )
                    .is_err(),
                    "{case}: {ty}: invalid supplied value was accepted"
                );
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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
