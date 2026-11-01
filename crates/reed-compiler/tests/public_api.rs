use reed_compiler::Schema;

#[test]
fn parses_and_validates_a_schema() {
    Schema::parse_and_validate("type Query { value: String }", "schema.graphql").unwrap();
}

#[test]
fn rejects_cyclic_defaults_without_aborting() {
    const CHILD_ENV: &str = "REED_COMPILER_CYCLIC_DEFAULT_TEST_CASE";
    let cases = [
        "input R { next: R = {} } type Query { value(input: R): String }",
        "input R { next: R = {} } type Query { value(input: R = {}): String }",
        "input A { bs: [B] = [{}] } input B { a: A = {} } type Query { value(input: A): String }",
    ];

    if let Ok(index) = std::env::var(CHILD_ENV) {
        let errors =
            Schema::parse_and_validate(cases[index.parse::<usize>().unwrap()], "schema.graphql")
                .expect_err("cyclic defaults must fail schema validation");
        assert!(errors.errors.iter().any(|diagnostic| {
            diagnostic
                .to_json()
                .message
                .contains("defaults contain a cycle")
        }));
        return;
    }

    for (index, _) in cases.iter().enumerate() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "rejects_cyclic_defaults_without_aborting",
                "--nocapture",
            ])
            .env(CHILD_ENV, index.to_string())
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
}
