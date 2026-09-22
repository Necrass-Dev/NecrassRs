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

        let value = coerce_json_input_value(schema, &definition.ty, value).map_err(|error| {
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
        Type::List(inner) | Type::NonNullList(inner) => coerce_json_list(schema, inner, value),
        Type::Named(name) | Type::NonNullNamed(name) => {
            coerce_json_named_value(schema, name, value)
        }
    }
}

fn coerce_json_list(
    schema: &Valid<Schema>,
    item_type: &Type,
    value: JsonValue,
) -> Result<JsonValue, InputCoercionError> {
    let values = match value {
        JsonValue::Array(values) => values,
        value => vec![value],
    };

    values
        .into_iter()
        .map(|value| coerce_json_input_value(schema, item_type, value))
        .collect::<Result<Vec<_>, _>>()
        .map(JsonValue::Array)
}

fn coerce_json_named_value(
    schema: &Valid<Schema>,
    type_name: &Name,
    value: JsonValue,
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
            Some(value) => value,
            None => {
                let Some(default) = &field_definition.default_value else {
                    continue;
                };

                default_value_to_json(default)?
            }
        };

        object.insert(
            field_name.as_str(),
            coerce_json_input_value(schema, &field_definition.ty, value)?,
        );
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
