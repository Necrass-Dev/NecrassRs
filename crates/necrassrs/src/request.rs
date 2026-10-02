use apollo_compiler::{
    ExecutableDocument, Node,
    ast::Document,
    executable::Operation,
    request::coerce_variable_values,
    response::{GraphQLError, JsonMap},
    validation::Valid,
};

/// An owned GraphQL document, optional operation name, and variable values.
///
/// Construction does not validate input. [`crate::execute`] parses and validates
/// the document, selects an operation, and coerces variables before dispatch.
///
/// ```
/// use necrassrs::{JsonMap, Request};
/// let mut variables = JsonMap::new();
/// variables.insert("name", "Sheri".into());
/// let request = Request::new("query Greeting($name: String!) { hello(name: $name) }")
///     .with_operation_name("Greeting")
///     .with_variables(variables);
/// ```
pub struct Request {
    document: String,
    operation_name: Option<String>,
    variables: JsonMap,
}

impl Request {
    /// Stores a document with no operation name and an empty variable map.
    pub fn new(document: impl Into<String>) -> Self {
        Self {
            document: document.into(),
            operation_name: None,
            variables: JsonMap::default(),
        }
    }

    /// Selects an operation by name, replacing any previous selection.
    ///
    /// Required when the document contains multiple operations. Unknown names
    /// produce request errors during execution.
    pub fn with_operation_name(mut self, operation_name: impl Into<String>) -> Self {
        self.operation_name = Some(operation_name.into());
        self
    }

    /// Replaces all variable values. Keys are variable names without `$`.
    ///
    /// Omitted entries and explicit JSON nulls remain distinct during coercion.
    pub fn with_variables(mut self, variables: JsonMap) -> Self {
        self.variables = variables;
        self
    }
}

#[derive(Debug)]
pub(crate) struct PreparedRequest {
    pub(crate) document: Valid<ExecutableDocument>,
    pub(crate) operation: Node<Operation>,
    pub(crate) variables: Valid<JsonMap>,
}

#[derive(Debug)]
pub(crate) struct RequestError {
    pub(crate) errors: Vec<GraphQLError>,
    pub(crate) syntax_error: bool,
}

impl From<Vec<GraphQLError>> for RequestError {
    fn from(errors: Vec<GraphQLError>) -> Self {
        Self {
            errors,
            syntax_error: false,
        }
    }
}

pub(crate) fn prepare_request(
    schema: &Valid<apollo_compiler::Schema>,
    request: &Request,
) -> Result<PreparedRequest, RequestError> {
    // Parse once into Apollo's AST so syntax and validation failures remain distinct.
    let ast = Document::parse(request.document.as_str(), "request.graphql").map_err(|error| {
        RequestError {
            errors: error
                .errors
                .iter()
                .map(|diagnostic| diagnostic.to_json())
                .collect(),
            syntax_error: true,
        }
    })?;
    let document = ast.to_executable_validate(schema).map_err(|error| {
        error
            .errors
            .iter()
            .map(|diagnostic| diagnostic.to_json())
            .collect::<Vec<_>>()
    })?;

    let operation = document
        .operations
        .get(request.operation_name.as_deref())
        .map_err(|error| vec![error.to_graphql_error(&document.sources)])?;

    let variables = coerce_variable_values(schema, operation, &request.variables)
        .map_err(|error| vec![error.to_graphql_error(&document.sources)])?;
    let operation = operation.clone();

    Ok(PreparedRequest {
        document,
        operation,
        variables,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use apollo_compiler::Schema;
    use apollo_compiler::response::serde_json_bytes::json;

    const MULTIPLE_OPERATIONS: &str = r#"
        query First {
            first
        }

        query Second {
            second
        }
    "#;

    fn operation_schema() -> Valid<Schema> {
        Schema::parse_and_validate(
            "type Query { first: String second: String }",
            "schema.graphql",
        )
        .unwrap()
    }

    #[test]
    fn one_of_schema_accepts_nullable_fields_without_defaults() {
        Schema::parse_and_validate(
            r#"
                directive @oneOf on INPUT_OBJECT
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
                "directive @oneOf on INPUT_OBJECT \
                 input Choice @oneOf {{ selected: {field_type} }} \
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
                "directive @oneOf on INPUT_OBJECT \
                 input Choice @oneOf {{ selected: Int = {default} }} \
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
            let document = super::Document::parse(source, "request.graphql").unwrap();
            document
                .to_executable_validate(&schema)
                .unwrap_or_else(|errors| {
                    panic!("valid OneOf document rejected: {source}\n{errors}")
                });
        }
    }

    #[test]
    fn one_of_document_rejects_invalid_selections_before_variable_coercion() {
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
            let document = super::Document::parse(source, "request.graphql").unwrap();
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

    fn one_of_document_schema() -> Valid<Schema> {
        Schema::parse_and_validate(
            r#"
                directive @oneOf on INPUT_OBJECT
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

    #[test]
    fn cyclic_input_default_is_rejected_without_aborting() {
        const CHILD_ENV: &str = "NECRASSRS_CYCLIC_DEFAULT_TEST_CHILD";

        if std::env::var_os(CHILD_ENV).is_some() {
            let errors = Schema::parse_and_validate(
                "input Recursive { next: Recursive = {} } \
                 type Query { hello(input: Recursive): String }",
                "schema.graphql",
            )
            .expect_err("cyclic input defaults must be rejected during schema validation");
            assert!(!errors.errors.is_empty());
            return;
        }

        // Stack overflow aborts cannot be caught inside the test runner.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "request::tests::cyclic_input_default_is_rejected_without_aborting",
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
            String::from_utf8_lossy(&output.stderr),
        );
    }

    #[test]
    fn cyclic_argument_defaults_are_rejected_before_execution() {
        const CHILD_ENV: &str = "NECRASSRS_CYCLIC_EXECUTION_TEST_CASE";
        let cases = [
            (
                "literal input",
                "input R { next: R = {} } type Query { hello(input: R): String }",
                "{ greeting: hello(input: {}) }",
            ),
            (
                "omitted argument",
                "input R { next: R = {} } type Query { hello(input: R = {}): String }",
                "{ greeting: hello }",
            ),
            (
                "missing argument variable",
                "input R { next: R = {} } type Query { hello(input: R = {}): String }",
                "query($input: R) { greeting: hello(input: $input) }",
            ),
            (
                "missing input field variable",
                "input R { next: R = {} } type Query { hello(input: R): String }",
                "query($next: R) { greeting: hello(input: {next: $next}) }",
            ),
            (
                "mutual cycle through a list",
                "input A { bs: [B] = [{}] } input B { a: A = {} } \
                 type Query { hello(input: A): String }",
                "{ greeting: hello(input: {}) }",
            ),
        ];

        if let Ok(index) = std::env::var(CHILD_ENV) {
            let (case, source, document) = cases[index.parse::<usize>().unwrap()];
            // Reject the schema without preparing the request or invoking a dispatcher.
            let errors = Schema::parse_and_validate(source, "schema.graphql")
                .err()
                .unwrap_or_else(|| {
                    panic!("{case}: schema must be rejected before executing {document}")
                });
            assert!(!errors.errors.is_empty(), "{case}");
            return;
        }

        // Isolate each case so a regression causing stack overflow cannot abort the suite.
        let mut failures = Vec::new();
        for (index, (case, _, _)) in cases.iter().enumerate() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "request::tests::cyclic_argument_defaults_are_rejected_before_execution",
                    "--nocapture",
                ])
                .env(CHILD_ENV, index.to_string())
                .output()
                .unwrap();

            if !output.status.success() {
                failures.push(format!(
                    "{case}: {}\nstdout:\n{}\nstderr:\n{}",
                    output.status,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn syntax_error_is_rejected_during_request_preparation() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();

        let request = Request::new("query {");

        let errors = prepare_request(&schema, &request).unwrap_err();

        assert!(errors.syntax_error);
        assert!(!errors.errors.is_empty());
        assert!(!errors.errors[0].locations.is_empty());
    }

    #[test]
    fn multiple_operations_require_an_operation_name() {
        let schema = operation_schema();
        let request = Request::new(MULTIPLE_OPERATIONS);

        let errors = prepare_request(&schema, &request).unwrap_err();

        assert_eq!(errors.errors.len(), 1);
    }

    #[test]
    fn requested_operation_is_selected() {
        let schema = operation_schema();
        let request = Request::new(MULTIPLE_OPERATIONS).with_operation_name("Second");

        let prepared = prepare_request(&schema, &request).unwrap();

        assert_eq!(prepared.operation.name.as_deref(), Some("Second"));
    }

    #[test]
    fn unknown_operation_name_is_rejected() {
        let schema = operation_schema();
        let request = Request::new(MULTIPLE_OPERATIONS).with_operation_name("Missing");

        let errors = prepare_request(&schema, &request).unwrap_err();

        assert_eq!(errors.errors.len(), 1);
    }

    #[test]
    fn missing_required_variable_is_rejected() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();

        let request = Request::new(
            r#"
                query Greeting($name: String!) {
                    hello(name: $name)
                }
            "#,
        );

        let errors = prepare_request(&schema, &request).unwrap_err();

        assert_eq!(errors.errors.len(), 1);
        assert!(!errors.errors[0].message.is_empty());
        assert!(!errors.errors[0].locations.is_empty());
        assert!(errors.errors[0].path.is_empty());
    }

    #[test]
    fn variable_values_are_coerced_during_request_preparation() {
        let schema = Schema::parse_and_validate(
            "type Query { greet(names: [String!]!): String! }",
            "schema.graphql",
        )
        .unwrap();

        let variables = json!({
            "names": "셰리"
        })
        .as_object()
        .unwrap()
        .clone();

        let request = Request::new(
            r#"
                query Greeting($names: [String!]!) {
                    greet(names: $names)
                }
            "#,
        )
        .with_variables(variables);

        let prepared = prepare_request(&schema, &request).unwrap();

        assert_eq!(prepared.variables.get("names"), Some(&json!(["셰리"])));
    }
}
