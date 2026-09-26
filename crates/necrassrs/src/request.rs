use std::collections::HashSet;

use apollo_compiler::{
    ExecutableDocument, Name, Node, Schema,
    ast::Type,
    executable::Operation,
    request::coerce_variable_values,
    response::{GraphQLError, JsonMap, JsonValue},
    schema::ExtendedType,
    validation::Valid,
};

use crate::input::{InputCoercionError, literal_to_json as default_value_to_json};

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
    let mut variables = variables.into_inner();

    for definition in &operation.variables {
        let name = definition.name.as_str();

        let Some(value) = variables.remove(name) else {
            continue;
        };

        let value = coerce_json_input_value(schema, &definition.ty, value, &mut HashSet::new())
            .map_err(|error| {
                vec![GraphQLError::new(
                    error.message,
                    error.location.or_else(|| definition.location()),
                    &document.sources,
                )]
            })?;

        variables.insert(name, value);
    }

    // Apollo already validated these values before normalization.
    let variables = Valid::assume_valid(variables);

    let operation = operation.clone();

    Ok(PreparedRequest {
        document,
        operation,
        variables,
    })
}

fn coerce_json_input_value(
    schema: &Valid<Schema>,
    ty: &Type,
    value: JsonValue,
    active_defaults: &mut HashSet<(Name, Name)>,
) -> Result<JsonValue, InputCoercionError> {
    if value.is_null() {
        return if ty.is_non_null() {
            Err(InputCoercionError::new(format!(
                "null value for non-null type '{ty}'"
            )))
        } else {
            Ok(JsonValue::Null)
        };
    }

    match ty {
        Type::List(inner) | Type::NonNullList(inner) => {
            coerce_json_list(schema, inner, value, active_defaults)
        }
        Type::Named(name) | Type::NonNullNamed(name) => {
            coerce_json_named_value(schema, name, value, active_defaults)
        }
    }
}

fn coerce_json_list(
    schema: &Valid<Schema>,
    item_type: &Type,
    value: JsonValue,
    active_defaults: &mut HashSet<(Name, Name)>,
) -> Result<JsonValue, InputCoercionError> {
    let values = match value {
        JsonValue::Array(values) => values,
        value => vec![value],
    };

    values
        .into_iter()
        .map(|value| coerce_json_input_value(schema, item_type, value, active_defaults))
        .collect::<Result<Vec<_>, _>>()
        .map(JsonValue::Array)
}

fn coerce_json_named_value(
    schema: &Valid<Schema>,
    type_name: &Name,
    value: JsonValue,
    active_defaults: &mut HashSet<(Name, Name)>,
) -> Result<JsonValue, InputCoercionError> {
    let Some(ExtendedType::InputObject(input)) = schema.types.get(type_name) else {
        // Scalar와 Enum은 Apollo가 이미 검증하고 coercion했습니다.
        return Ok(value);
    };

    let JsonValue::Object(mut object) = value else {
        return Err(InputCoercionError::new(format!(
            "could not coerce value to input object '{type_name}'"
        )));
    };

    for (field_name, field_definition) in &input.fields {
        let value = match object.remove(field_name.as_str()) {
            Some(value) => {
                coerce_json_input_value(schema, &field_definition.ty, value, active_defaults)?
            }
            None => {
                let Some(default) = &field_definition.default_value else {
                    continue;
                };

                let coordinate = (type_name.clone(), field_name.clone());
                if !active_defaults.insert(coordinate.clone()) {
                    return Err(InputCoercionError::at(
                        format!("cyclic default value for input field '{type_name}.{field_name}'"),
                        default.location(),
                    ));
                }

                let result = default_value_to_json(default).and_then(|value| {
                    coerce_json_input_value(schema, &field_definition.ty, value, active_defaults)
                });

                active_defaults.remove(&coordinate);

                result?
            }
        };

        object.insert(field_name.as_str(), value);
    }

    Ok(JsonValue::Object(object))
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
    fn cyclic_input_default_is_rejected_without_aborting() {
        const CHILD_ENV: &str = "NECRASSRS_CYCLIC_DEFAULT_TEST_CHILD";

        if std::env::var_os(CHILD_ENV).is_some() {
            let Ok(schema) = Schema::parse_and_validate(
                "input Recursive { next: Recursive = {} } \
                 type Query { hello(input: Recursive): String }",
                "schema.graphql",
            ) else {
                // Rejecting the cycle during schema validation is also safe.
                return;
            };
            let request = Request::new("query($input: Recursive = {}) { hello(input: $input) }");

            let errors = prepare_request(&schema, &request)
                .expect_err("cyclic defaults must be rejected before execution");
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
