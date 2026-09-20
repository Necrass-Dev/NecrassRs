use apollo_compiler::{
    ExecutableDocument, Node,
    executable::Operation,
    request::coerce_variable_values,
    response::{GraphQLError, JsonMap},
    validation::Valid,
};

pub struct Request {
    document: String,
    operation_name: Option<String>,
    variables: JsonMap,
}

impl Request {
    pub fn new(document: impl Into<String>) -> Self {
        Self {
            document: document.into(),
            operation_name: None,
            variables: JsonMap::default(),
        }
    }

    pub fn with_operation_name(mut self, operation_name: impl Into<String>) -> Self {
        self.operation_name = Some(operation_name.into());
        self
    }

    pub fn with_variables(mut self, variables: JsonMap) -> Self {
        self.variables = variables;
        self
    }
}

#[derive(Debug)]
pub(crate) struct PreparedRequest {
    #[expect(dead_code, reason = "used by execution once it is added")]
    document: Valid<ExecutableDocument>,
    operation: Node<Operation>,
    variables: Valid<JsonMap>,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the execution entry point once it is added"
    )
)]
pub(crate) fn prepare_request(
    schema: &Valid<apollo_compiler::Schema>,
    request: &Request,
) -> Result<PreparedRequest, Vec<GraphQLError>> {
    let document = ExecutableDocument::parse_and_validate(
        schema,
        request.document.as_str(),
        "request.graphql",
    )
    .map_err(|error| {
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
    fn syntax_error_is_rejected_during_request_preparation() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();

        let request = Request::new("query {");

        let errors = prepare_request(&schema, &request).unwrap_err();

        assert!(!errors.is_empty());
        assert!(!errors[0].locations.is_empty());
    }

    #[test]
    fn multiple_operations_require_an_operation_name() {
        let schema = operation_schema();
        let request = Request::new(MULTIPLE_OPERATIONS);

        let errors = prepare_request(&schema, &request).unwrap_err();

        assert_eq!(errors.len(), 1);
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

        assert_eq!(errors.len(), 1);
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

        assert_eq!(errors.len(), 1);
        assert!(!errors[0].message.is_empty());
        assert!(!errors[0].locations.is_empty());
        assert!(errors[0].path.is_empty());
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
