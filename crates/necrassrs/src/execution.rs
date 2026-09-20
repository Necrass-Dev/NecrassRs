use apollo_compiler::{
    Name, Node, Schema,
    ast::{Type, Value},
    collections::IndexMap,
    executable::{Field, Selection},
    parser::SourceSpan,
    response::{GraphQLError, JsonMap, JsonValue},
    schema::ExtendedType,
    validation::Valid,
};

use crate::request::PreparedRequest;

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the execution entry point once it is added"
    )
)]
fn collect_fields(prepared: &PreparedRequest) -> IndexMap<Name, Vec<&Field>> {
    prepared
        .operation
        .selection_set
        .selections
        .iter()
        .filter_map(|selection| match selection {
            Selection::Field(field) => Some(field.as_ref()),
            _ => None,
        })
        .fold(IndexMap::default(), |mut fields, field| {
            fields
                .entry(field.response_key().clone())
                .or_default()
                .push(field);
            fields
        })
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by field execution once it is added")
)]
fn coerce_argument_values(
    schema: &Valid<Schema>,
    prepared: &PreparedRequest,
    field: &Field,
) -> Result<JsonMap, Box<GraphQLError>> {
    field
        .definition
        .arguments
        .iter()
        .try_fold(JsonMap::new(), |mut coerced_values, definition| {
            let argument_value = field
                .specified_argument_by_name(definition.name.as_str())
                .or(definition.default_value.as_ref());
            let Some(argument_value) = argument_value else {
                if definition.ty.is_non_null() {
                    return Err(new_coercion_error(
                        prepared,
                        format!("missing value for required argument '{}'", definition.name),
                        definition.location(),
                    ));
                }

                return Ok(coerced_values);
            };

            let value = if let Some(variable_name) = argument_value.as_variable() {
                match prepared.variables.get(variable_name.as_str()) {
                    Some(variable_value)
                        if variable_value.is_null() && definition.ty.is_non_null() =>
                    {
                        return Err(new_coercion_error(
                            prepared,
                            format!("null value for non-null argument '{}'", definition.name),
                            argument_value.location(),
                        ));
                    }
                    Some(variable_value) => variable_value.clone(),
                    None => match definition.default_value.as_ref() {
                        Some(default_value) => {
                            coerce_input_value(schema, prepared, &definition.ty, default_value)?
                        }
                        None if definition.ty.is_non_null() => {
                            return Err(new_coercion_error(
                                prepared,
                                format!(
                                    "missing value for required argument '{}'",
                                    definition.name
                                ),
                                argument_value.location(),
                            ));
                        }
                        None => return Ok(coerced_values),
                    },
                }
            } else {
                coerce_input_value(schema, prepared, &definition.ty, argument_value)?
            };

            coerced_values.insert(definition.name.as_str(), value);
            Ok(coerced_values)
        })
}

fn coerce_input_value(
    schema: &Valid<Schema>,
    prepared: &PreparedRequest,
    ty: &Type,
    value: &Node<Value>,
) -> Result<JsonValue, Box<GraphQLError>> {
    if let Some(variable_name) = value.as_variable() {
        return match prepared.variables.get(variable_name.as_str()) {
            Some(variable_value) if variable_value.is_null() && ty.is_non_null() => {
                Err(new_coercion_error(
                    prepared,
                    format!("null variable '${variable_name}' for non-null type '{ty}'"),
                    value.location(),
                ))
            }
            Some(variable_value) => Ok(variable_value.clone()),
            None if ty.is_non_null() => Err(new_coercion_error(
                prepared,
                format!("missing variable '${variable_name}' for non-null type '{ty}'"),
                value.location(),
            )),
            None => Ok(JsonValue::Null),
        };
    }

    if value.is_null() {
        return if ty.is_non_null() {
            Err(new_coercion_error(
                prepared,
                format!("null value for non-null type '{ty}'"),
                value.location(),
            ))
        } else {
            Ok(JsonValue::Null)
        };
    }

    let type_name = match ty {
        Type::List(inner) | Type::NonNullList(inner) => {
            return value
                .as_list()
                .unwrap_or(std::slice::from_ref(value))
                .iter()
                .map(|value| coerce_input_value(schema, prepared, inner, value))
                .collect::<Result<Vec<_>, _>>()
                .map(Into::into);
        }
        Type::Named(name) | Type::NonNullNamed(name) => name,
    };

    if let Some(ExtendedType::InputObject(input)) = schema.types.get(type_name) {
        let object = value.as_object().ok_or_else(|| {
            new_coercion_error(
                prepared,
                format!("could not coerce value to input object '{type_name}'"),
                value.location(),
            )
        })?;

        return input
            .fields
            .iter()
            .try_fold(JsonMap::new(), |mut coerced, (name, definition)| {
                let specified = object
                    .iter()
                    .find(|(field_name, _)| field_name == name)
                    .map(|(_, value)| value);
                let value = match specified {
                    Some(value)
                        if value.as_variable().is_some_and(|variable_name| {
                            !prepared.variables.contains_key(variable_name.as_str())
                        }) =>
                    {
                        definition.default_value.as_ref()
                    }
                    Some(value) => Some(value),
                    None => definition.default_value.as_ref(),
                };

                match value {
                    Some(value) => {
                        coerced.insert(
                            name.as_str(),
                            coerce_input_value(schema, prepared, &definition.ty, value)?,
                        );
                    }
                    None if definition.ty.is_non_null() => {
                        return Err(new_coercion_error(
                            prepared,
                            format!("missing value for required input field '{type_name}.{name}'"),
                            definition.location(),
                        ));
                    }
                    None => {}
                }

                Ok(coerced)
            })
            .map(Into::into);
    }

    literal_to_json(prepared, value)
}

fn literal_to_json(
    prepared: &PreparedRequest,
    value: &Node<Value>,
) -> Result<JsonValue, Box<GraphQLError>> {
    match value.as_ref() {
        Value::Null => Ok(JsonValue::Null),
        Value::Enum(value) => Ok(value.as_str().into()),
        Value::String(value) => Ok(value.as_str().into()),
        Value::Boolean(value) => Ok((*value).into()),
        Value::Int(number) => number.as_str().parse().map(JsonValue::Number).map_err(|_| {
            new_coercion_error(
                prepared,
                "integer argument is outside the supported JSON range",
                value.location(),
            )
        }),
        Value::Float(number) => number.as_str().parse().map(JsonValue::Number).map_err(|_| {
            new_coercion_error(
                prepared,
                "float argument is outside the supported JSON range",
                value.location(),
            )
        }),
        Value::List(values) => values
            .iter()
            .map(|value| literal_to_json(prepared, value))
            .collect::<Result<Vec<_>, _>>()
            .map(Into::into),
        Value::Object(values) => values
            .iter()
            .map(|(name, value)| Ok((name.as_str().into(), literal_to_json(prepared, value)?)))
            .collect::<Result<JsonMap, _>>()
            .map(Into::into),
        Value::Variable(name) => Err(new_coercion_error(
            prepared,
            format!("unresolved variable '${name}'"),
            value.location(),
        )),
    }
}

fn new_coercion_error(
    prepared: &PreparedRequest,
    message: impl Into<String>,
    location: Option<SourceSpan>,
) -> Box<GraphQLError> {
    Box::new(GraphQLError::new(
        message,
        location,
        &prepared.document.sources,
    ))
}

#[cfg(test)]
mod tests {
    use crate::{Request, request::prepare_request};
    use apollo_compiler::{Schema, response::JsonMap, response::serde_json_bytes::json};

    fn coerce_arguments(schema_source: &str, request: Request) -> JsonMap {
        let schema = Schema::parse_and_validate(schema_source, "schema.graphql").unwrap();
        let prepared = prepare_request(&schema, &request).unwrap();
        let field = super::collect_fields(&prepared)
            .into_values()
            .next()
            .unwrap()[0];

        super::coerce_argument_values(&schema, &prepared, field).unwrap()
    }

    #[test]
    fn direct_root_field_is_collected_by_response_key() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();

        let request = Request::new(
            r#"
                query {
                    greeting: hello(name: "Sheri")
                }
            "#,
        );

        let prepared = prepare_request(&schema, &request).unwrap();

        let fields = super::collect_fields(&prepared);
        let collected = fields.get("greeting").unwrap();

        assert_eq!(fields.len(), 1);
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].name.as_str(), "hello");
    }

    #[test]
    fn literal_argument_is_coerced_for_resolver_dispatch() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();

        let request = Request::new(
            r#"
                query {
                    hello(name: "Sheri")
                }
            "#,
        );

        let prepared = prepare_request(&schema, &request).unwrap();
        let fields = super::collect_fields(&prepared);
        let field = fields.get("hello").unwrap()[0];

        let arguments = super::coerce_argument_values(&schema, &prepared, field).unwrap();

        assert_eq!(arguments.get("name"), Some(&json!("Sheri")));
    }

    #[test]
    fn coerced_variable_is_used_as_an_argument() {
        let variables = json!({ "name": "Sheri" }).as_object().unwrap().clone();
        let request = Request::new(
            r#"
                query Greeting($name: String!) {
                    hello(name: $name)
                }
            "#,
        )
        .with_variables(variables);

        let arguments = coerce_arguments("type Query { hello(name: String!): String! }", request);

        assert_eq!(arguments.get("name"), Some(&json!("Sheri")));
    }

    #[test]
    fn omitted_nullable_argument_is_absent_and_explicit_null_is_preserved() {
        let arguments = coerce_arguments(
            "type Query { hello(omitted: String, explicit: String): String! }",
            Request::new("query { hello(explicit: null) }"),
        );

        assert!(!arguments.contains_key("omitted"));
        assert_eq!(arguments.get("explicit"), Some(&json!(null)));
    }

    #[test]
    fn argument_default_is_used_when_the_argument_is_omitted() {
        let arguments = coerce_arguments(
            r#"type Query { hello(name: String! = "Sheri"): String! }"#,
            Request::new("query { hello }"),
        );

        assert_eq!(arguments.get("name"), Some(&json!("Sheri")));
    }

    #[test]
    fn input_object_defaults_and_single_value_list_coercion_are_applied() {
        let arguments = coerce_arguments(
            r#"
                input GreetingInput {
                    name: String!
                    titles: [String!]! = "Detective"
                }

                type Query {
                    hello(input: GreetingInput!): String!
                }
            "#,
            Request::new(r#"query { hello(input: { name: "Sheri" }) }"#),
        );

        assert_eq!(
            arguments.get("input"),
            Some(&json!({
                "name": "Sheri",
                "titles": ["Detective"]
            }))
        );
    }

    #[test]
    fn variable_inside_an_input_object_uses_its_coerced_value() {
        let variables = json!({ "name": "Sheri" }).as_object().unwrap().clone();
        let request = Request::new(
            r#"
                query Greeting($name: String!) {
                    hello(input: { name: $name })
                }
            "#,
        )
        .with_variables(variables);

        let arguments = coerce_arguments(
            r#"
                input GreetingInput { name: String! }
                type Query { hello(input: GreetingInput!): String! }
            "#,
            request,
        );

        assert_eq!(arguments.get("input"), Some(&json!({ "name": "Sheri" })));
    }
}
