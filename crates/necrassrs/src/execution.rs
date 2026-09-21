use apollo_compiler::{
    Name, Node, Schema,
    ast::{Type, Value},
    collections::IndexMap,
    executable::{Field, Selection},
    parser::SourceSpan,
    response::{GraphQLError, JsonMap, JsonValue, ResponseDataPathSegment},
    schema::ExtendedType,
    validation::Valid,
};

use crate::{ResolverError, request::PreparedRequest};

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by field execution once it is added")
)]
fn resolver_error_to_graphql_error(
    prepared: &PreparedRequest,
    field: &Field,
    path: &[ResponseDataPathSegment],
    resolver_error: ResolverError,
) -> Box<GraphQLError> {
    let ResolverError {
        message,
        extensions,
    } = resolver_error;

    let mut error = new_execution_error(prepared, path, message, field.name.location());

    error.extensions = extensions.unwrap_or_default();

    error
}

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
    path: &[ResponseDataPathSegment],
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
                    return Err(new_execution_error(
                        prepared,
                        path,
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
                        return Err(new_execution_error(
                            prepared,
                            path,
                            format!("null value for non-null argument '{}'", definition.name),
                            argument_value.location(),
                        ));
                    }
                    Some(variable_value) => variable_value.clone(),
                    None => match definition.default_value.as_ref() {
                        Some(default_value) => coerce_input_value(
                            schema,
                            prepared,
                            path,
                            &definition.ty,
                            default_value,
                        )?,
                        None if definition.ty.is_non_null() => {
                            return Err(new_execution_error(
                                prepared,
                                path,
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
                coerce_input_value(schema, prepared, path, &definition.ty, argument_value)?
            };

            coerced_values.insert(definition.name.as_str(), value);
            Ok(coerced_values)
        })
}

fn coerce_input_value(
    schema: &Valid<Schema>,
    prepared: &PreparedRequest,
    path: &[ResponseDataPathSegment],
    ty: &Type,
    value: &Node<Value>,
) -> Result<JsonValue, Box<GraphQLError>> {
    if let Some(variable_name) = value.as_variable() {
        return match prepared.variables.get(variable_name.as_str()) {
            Some(variable_value) if variable_value.is_null() && ty.is_non_null() => {
                Err(new_execution_error(
                    prepared,
                    path,
                    format!("null variable '${variable_name}' for non-null type '{ty}'"),
                    value.location(),
                ))
            }
            Some(variable_value) => Ok(variable_value.clone()),
            None if ty.is_non_null() => Err(new_execution_error(
                prepared,
                path,
                format!("missing variable '${variable_name}' for non-null type '{ty}'"),
                value.location(),
            )),
            None => Ok(JsonValue::Null),
        };
    }

    if value.is_null() {
        return if ty.is_non_null() {
            Err(new_execution_error(
                prepared,
                path,
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
                .map(|value| coerce_input_value(schema, prepared, path, inner, value))
                .collect::<Result<Vec<_>, _>>()
                .map(Into::into);
        }
        Type::Named(name) | Type::NonNullNamed(name) => name,
    };

    if let Some(ExtendedType::InputObject(input)) = schema.types.get(type_name) {
        let object = value.as_object().ok_or_else(|| {
            new_execution_error(
                prepared,
                path,
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
                            coerce_input_value(schema, prepared, path, &definition.ty, value)?,
                        );
                    }
                    None if definition.ty.is_non_null() => {
                        return Err(new_execution_error(
                            prepared,
                            path,
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

    literal_to_json(prepared, path, value)
}

fn literal_to_json(
    prepared: &PreparedRequest,
    path: &[ResponseDataPathSegment],
    value: &Node<Value>,
) -> Result<JsonValue, Box<GraphQLError>> {
    match value.as_ref() {
        Value::Null => Ok(JsonValue::Null),
        Value::Enum(value) => Ok(value.as_str().into()),
        Value::String(value) => Ok(value.as_str().into()),
        Value::Boolean(value) => Ok((*value).into()),
        Value::Int(number) => number.as_str().parse().map(JsonValue::Number).map_err(|_| {
            new_execution_error(
                prepared,
                path,
                "integer argument is outside the supported JSON range",
                value.location(),
            )
        }),
        Value::Float(number) => number.as_str().parse().map(JsonValue::Number).map_err(|_| {
            new_execution_error(
                prepared,
                path,
                "float argument is outside the supported JSON range",
                value.location(),
            )
        }),
        Value::List(values) => values
            .iter()
            .map(|value| literal_to_json(prepared, path, value))
            .collect::<Result<Vec<_>, _>>()
            .map(Into::into),
        Value::Object(values) => values
            .iter()
            .map(|(name, value)| {
                Ok((
                    name.as_str().into(),
                    literal_to_json(prepared, path, value)?,
                ))
            })
            .collect::<Result<JsonMap, _>>()
            .map(Into::into),
        Value::Variable(name) => Err(new_execution_error(
            prepared,
            path,
            format!("unresolved variable '${name}'"),
            value.location(),
        )),
    }
}

fn new_execution_error(
    prepared: &PreparedRequest,
    path: &[ResponseDataPathSegment],
    message: impl Into<String>,
    location: Option<SourceSpan>,
) -> Box<GraphQLError> {
    let mut error = GraphQLError::new(message, location, &prepared.document.sources);

    error.path = path.to_vec();

    Box::new(error)
}

#[cfg(test)]
mod tests {
    use crate::{Request, ResolverError, request::prepare_request};
    use apollo_compiler::{
        Schema,
        response::{
            JsonMap, JsonValue, ResponseDataPathSegment,
            serde_json_bytes::{json, to_value},
        },
    };

    struct TestContext<'a> {
        greeting: &'a str,
    }

    struct TestDispatcher;

    impl<'context> super::Dispatcher<TestContext<'context>> for TestDispatcher {
        fn resolve<'a>(
            &'a self,
            context: &'a TestContext<'context>,
            field_name: &'a str,
            arguments: &'a JsonMap,
        ) -> impl Future<Output = Result<JsonValue, ResolverError>> + Send + 'a {
            async move {
                assert_eq!(field_name, "hello");

                let name = arguments.get("name").and_then(JsonValue::as_str).unwrap();

                Ok(json!(format!("{}, {name}", context.greeting)))
            }
        }
    }

    fn assert_send<T: Send>(_: &T) {}

    fn coerce_arguments(schema_source: &str, request: Request) -> JsonMap {
        let schema = Schema::parse_and_validate(schema_source, "schema.graphql").unwrap();
        let prepared = prepare_request(&schema, &request).unwrap();
        let field = super::collect_fields(&prepared)
            .into_values()
            .next()
            .unwrap()[0];

        super::coerce_argument_values(&schema, &prepared, &[], field).unwrap()
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

        let arguments = super::coerce_argument_values(&schema, &prepared, &[], field).unwrap();

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

    #[test]
    fn argument_coercion_error_contains_alias_aware_path() {
        let schema = Schema::parse_and_validate(
            r#"
                type Query {
                    hello(name: String! = "Sheri"): String!
                }
            "#,
            "schema.graphql",
        )
        .unwrap();

        let variables = json!({ "name": null }).as_object().unwrap().clone();
        let request = Request::new(
            r#"
                query Greeting($name: String) {
                    greeting: hello(name: $name)
                }
            "#,
        )
        .with_variables(variables);

        let prepared = prepare_request(&schema, &request).unwrap();
        let field = super::collect_fields(&prepared).get("greeting").unwrap()[0];
        let path = vec![ResponseDataPathSegment::Field(field.response_key().clone())];

        let error = super::coerce_argument_values(&schema, &prepared, &path, field).unwrap_err();

        assert_eq!(error.path, path);
    }

    #[test]
    fn resolver_error_is_enriched_with_location_and_alias_aware_path() {
        let schema =
            Schema::parse_and_validate("type Query { hello: String }", "schema.graphql").unwrap();
        let request = Request::new(
            r#"
                query {
                    greeting: hello
                }
            "#,
        );

        let prepared = prepare_request(&schema, &request).unwrap();
        let field = super::collect_fields(&prepared).get("greeting").unwrap()[0];
        let path = vec![ResponseDataPathSegment::Field(field.response_key().clone())];
        let resolver_error =
            ResolverError::new("Greeting failed.").with_extension("code", "GREETING_FAILED");

        let error = super::resolver_error_to_graphql_error(&prepared, field, &path, resolver_error);

        assert_eq!(error.message, "Greeting failed.");
        assert_eq!(
            error.extensions.get("code"),
            Some(&json!("GREETING_FAILED"))
        );
        assert!(!error.locations.is_empty());
        assert_eq!(error.path, path);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn handwritten_dispatch_receives_coerced_arguments_and_borrowed_context() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();
        let request = Request::new(r#"query { hello(name: "Sheri") }"#);
        let greeting = String::from("Hello");
        let context = TestContext {
            greeting: &greeting,
        };
        let dispatcher = TestDispatcher;

        let future = super::execute(&schema, &request, &dispatcher, &context);

        assert_send(&future);

        let response = future.await;

        assert_eq!(
            to_value(response).unwrap(),
            json!({
                "data": {
                    "hello": "Hello, Sheri"
                }
            })
        );
    }
}
