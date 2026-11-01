use lead_compiler::{ast, Schema};

#[test]
fn parses_and_validates_a_schema() {
    Schema::parse_and_validate("type Query { value: String }", "schema.graphql").unwrap();
}

#[test]
fn exposes_maintained_parser_unicode_behavior() {
    for (literal, expected) in [(r#""\uD83D\uDE00""#, "😀"), (r#""\u{1F600}""#, "😀")] {
        let document = ast::Document::parse(
            format!("query Example($value: String = {literal}) {{ value }}"),
            "query.graphql",
        )
        .unwrap();
        let ast::Definition::OperationDefinition(operation) = &document.definitions[0] else {
            panic!("expected an operation definition")
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

    assert!(ast::Document::parse(
        "query Example($value: String = \"\\uD800\") { value }",
        "query.graphql"
    )
    .is_err());
}

#[test]
fn rejects_cyclic_input_defaults_without_aborting() {
    const CHILD_ENV: &str = "LEAD_COMPILER_CYCLIC_DEFAULT_TEST_CHILD";

    if std::env::var_os(CHILD_ENV).is_some() {
        let errors = Schema::parse_and_validate(
            "input Recursive { next: Recursive = {} } type Query { value(input: Recursive): String }",
            "schema.graphql",
        )
        .expect_err("cyclic input defaults must fail schema validation");
        assert!(errors.errors.iter().any(|diagnostic| {
            diagnostic
                .to_json()
                .message
                .contains("defaults contain a cycle")
        }));
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "rejects_cyclic_input_defaults_without_aborting",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cyclic default validation failed ({})\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
