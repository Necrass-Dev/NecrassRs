use reed_compiler::{
    ExecutableDocument, Node,
    ast::{Definition, Document, OperationType},
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

    /// Returns the selected operation type when syntax and operation selection are unambiguous.
    ///
    /// Schema validation and variable coercion remain part of execution.
    pub fn operation_type(&self) -> Option<OperationType> {
        let document = Document::parse(self.document.as_str(), "request.graphql").ok()?;
        let mut operations = document.definitions.iter().filter_map(|definition| {
            let Definition::OperationDefinition(operation) = definition else {
                return None;
            };
            match self.operation_name.as_deref() {
                Some(name) if operation.name.as_deref() == Some(name) => Some(operation),
                Some(_) => None,
                None => Some(operation),
            }
        });
        let operation = operations.next()?;
        operations
            .next()
            .is_none()
            .then_some(operation.operation_type)
    }
}

#[derive(Debug)]
pub(crate) struct PreparedRequest {
    pub(crate) document: Valid<ExecutableDocument>,
    pub(crate) operation: Node<Operation>,
    pub(crate) provided_variables: JsonMap,
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
    schema: &Valid<reed_compiler::Schema>,
    request: &Request,
) -> Result<PreparedRequest, RequestError> {
    // Parse once into Reed's AST so syntax and validation failures remain distinct.
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
        provided_variables: request.variables.clone(),
        variables,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reed_compiler::Schema;
    use reed_compiler::response::serde_json_bytes::json;

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
    fn reports_only_an_unambiguously_selected_operation_type() {
        assert_eq!(
            Request::new("query First { first } mutation Second { second }")
                .with_operation_name("Second")
                .operation_type(),
            Some(OperationType::Mutation)
        );
        assert_eq!(
            Request::new("query First { first } query Second { second }").operation_type(),
            None
        );
        assert_eq!(Request::new("query {").operation_type(), None);
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
