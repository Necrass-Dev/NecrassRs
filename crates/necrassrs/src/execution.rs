use apollo_compiler::{
    Name, Node, Schema,
    ast::{Type, Value},
    collections::{HashSet, IndexMap},
    executable::{Field, Selection},
    parser::SourceSpan,
    response::{GraphQLError, JsonMap, JsonValue, ResponseDataPathSegment},
    schema::{ExtendedType, ObjectType},
    validation::Valid,
};

use crate::{
    Request, ResolverError, Response,
    request::{PreparedRequest, prepare_request},
};

pub trait Dispatcher<C> {
    fn resolve<'a>(
        &'a self,
        context: &'a C,
        field_name: &'a str,
        arguments: &'a JsonMap,
    ) -> impl Future<Output = Result<JsonValue, ResolverError>> + Send + 'a;
}

pub async fn execute<C, D>(
    schema: &Valid<Schema>,
    request: &Request,
    dispatcher: &D,
    context: &C,
) -> Response
where
    C: Sync,
    D: Dispatcher<C> + Sync,
{
    let prepared = match prepare_request(schema, request) {
        Ok(prepared) => prepared,
        Err(errors) => return Response::request_errors(errors),
    };

    let mut data = JsonMap::new();
    let mut errors = Vec::new();

    for (response_key, fields) in collect_fields(schema, &prepared) {
        // TODO: Expand next line, 필드 병합 테스트가 변경을 요구할 때 구현.
        let field = fields[0];
        let mut path = vec![ResponseDataPathSegment::Field(response_key.clone())];

        let (value, has_error) = match coerce_argument_values(schema, &prepared, &path, field) {
            Ok(arguments) => match dispatcher
                .resolve(context, field.name.as_str(), &arguments)
                .await
            {
                Ok(value) => (value, false),
                Err(error) => {
                    errors.push(*resolver_error_to_graphql_error(
                        &prepared, field, &path, error,
                    ));

                    (JsonValue::Null, true)
                }
            },
            Err(error) => {
                errors.push(*error);
                (JsonValue::Null, true)
            }
        };

        if value.is_null() && field.definition.ty.is_non_null() {
            if !has_error {
                errors.push(*new_execution_error(
                    &prepared,
                    &path,
                    format!(
                        "Cannot return null for non-nullable field '{}'.",
                        field.name
                    ),
                    field.name.location(),
                ));
            }
            return Response::execution(None, errors);
        }

        let value = match complete_value(
            &prepared,
            field,
            &field.definition.ty,
            value,
            &mut path,
            &mut errors,
        ) {
            Ok(value) => value,
            Err(PropagateNull) => return Response::execution(None, errors),
        };

        data.insert(response_key.as_str(), value);
    }

    Response::execution(Some(data), errors)
}

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

fn collect_fields<'a>(
    schema: &Valid<Schema>,
    prepared: &'a PreparedRequest,
) -> IndexMap<Name, Vec<&'a Field>> {
    let mut fields = IndexMap::default();
    let mut visited_fragments = HashSet::default();
    let object_type = schema
        .get_object(prepared.operation.selection_set.ty.as_str())
        .expect("validated operation root must be an object");

    collect_selections(
        schema,
        object_type,
        prepared,
        &prepared.operation.selection_set.selections,
        &mut visited_fragments,
        &mut fields,
    );

    fields
}

fn collect_selections<'a>(
    schema: &Valid<Schema>,
    object_type: &ObjectType,
    prepared: &'a PreparedRequest,
    selections: &'a [Selection],
    visited_fragments: &mut HashSet<&'a Name>,
    fields: &mut IndexMap<Name, Vec<&'a Field>>,
) {
    selections
        .iter()
        .filter(|selection| should_include(selection, &prepared.variables))
        .for_each(|selection| match selection {
            Selection::Field(field) => {
                fields
                    .entry(field.response_key().clone())
                    .or_default()
                    .push(field);
            }
            Selection::FragmentSpread(spread) => {
                if visited_fragments.insert(&spread.fragment_name)
                    && let Some(fragment) = prepared.document.fragments.get(&spread.fragment_name)
                    && does_fragment_type_apply(schema, object_type, fragment.type_condition())
                {
                    collect_selections(
                        schema,
                        object_type,
                        prepared,
                        &fragment.selection_set.selections,
                        visited_fragments,
                        fields,
                    );
                }
            }
            Selection::InlineFragment(fragment) => {
                if fragment.type_condition.as_ref().is_none_or(|condition| {
                    does_fragment_type_apply(schema, object_type, condition)
                }) {
                    collect_selections(
                        schema,
                        object_type,
                        prepared,
                        &fragment.selection_set.selections,
                        visited_fragments,
                        fields,
                    );
                }
            }
        });
}

fn does_fragment_type_apply(
    schema: &Valid<Schema>,
    object_type: &ObjectType,
    condition: &Name,
) -> bool {
    match schema.types.get(condition) {
        Some(ExtendedType::Object(_)) => condition == &object_type.name,
        Some(ExtendedType::Interface(_)) => object_type.implements_interfaces.contains(condition),
        Some(ExtendedType::Union(union)) => union.members.contains(&object_type.name),
        _ => false,
    }
}

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

struct PropagateNull;

fn complete_value(
    prepared: &PreparedRequest,
    field: &Field,
    ty: &Type,
    value: JsonValue,
    path: &mut Vec<ResponseDataPathSegment>,
    errors: &mut Vec<GraphQLError>,
) -> Result<JsonValue, PropagateNull> {
    if value.is_null() {
        return if ty.is_non_null() {
            errors.push(*new_execution_error(
                prepared,
                path,
                format!(
                    "Cannot return null for non-nullable field '{}'.",
                    field.name
                ),
                field.name.location(),
            ));

            Err(PropagateNull)
        } else {
            Ok(JsonValue::Null)
        };
    }

    match ty {
        Type::List(item_type) | Type::NonNullList(item_type) => {
            let JsonValue::Array(values) = value else {
                errors.push(*new_execution_error(
                    prepared,
                    path,
                    format!("Expected field '{}' to return a list.", field.name),
                    field.name.location(),
                ));

                return if ty.is_non_null() {
                    Err(PropagateNull)
                } else {
                    Ok(JsonValue::Null)
                };
            };

            let completed = values
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    path.push(ResponseDataPathSegment::ListIndex(index));

                    let completed = complete_value(prepared, field, item_type, value, path, errors);

                    path.pop();

                    completed
                })
                .collect::<Result<Vec<_>, _>>();

            match completed {
                Ok(values) => Ok(values.into()),
                Err(PropagateNull) if ty.is_non_null() => Err(PropagateNull),
                Err(PropagateNull) => Ok(JsonValue::Null),
            }
        }
        Type::Named(name) | Type::NonNullNamed(name) => {
            if name.as_str() == "String" && !value.is_string() {
                errors.push(*new_execution_error(
                    prepared,
                    path,
                    format!("Expected field '{}' to return a String.", field.name),
                    field.name.location(),
                ));

                return if ty.is_non_null() {
                    Err(PropagateNull)
                } else {
                    Ok(JsonValue::Null)
                };
            }

            Ok(value)
        }
    }
}

fn directive_condition(selection: &Selection, name: &str, variables: &JsonMap) -> Option<bool> {
    let value = selection
        .directives()
        .get(name)?
        .specified_argument_by_name("if")?;

    match value.as_ref() {
        Value::Boolean(value) => Some(*value),
        Value::Variable(name) => variables.get(name.as_str())?.as_bool(),
        _ => None,
    }
}

fn should_include(selection: &Selection, variables: &JsonMap) -> bool {
    !directive_condition(selection, "skip", variables).unwrap_or(false)
        && directive_condition(selection, "include", variables).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    struct FailingDispatcher;

    struct NullDispatcher;

    struct ValueDispatcher(JsonValue);

    struct CountingDispatcher(AtomicUsize);

    struct RecoveringDispatcher(AtomicUsize);

    impl<'context> super::Dispatcher<TestContext<'context>> for TestDispatcher {
        async fn resolve<'a>(
            &'a self,
            context: &'a TestContext<'context>,
            field_name: &'a str,
            arguments: &'a JsonMap,
        ) -> Result<JsonValue, ResolverError> {
            tokio::task::yield_now().await;

            assert_eq!(field_name, "hello");

            let name = arguments.get("name").and_then(JsonValue::as_str).unwrap();

            Ok(json!(format!("{}, {name}", context.greeting)))
        }
    }

    impl super::Dispatcher<()> for FailingDispatcher {
        async fn resolve<'a>(
            &'a self,
            _context: &'a (),
            _field_name: &'a str,
            _arguments: &'a JsonMap,
        ) -> Result<JsonValue, ResolverError> {
            Err(ResolverError::new("Greeting failed.").with_extension("code", "GREETING_FAILED"))
        }
    }

    impl super::Dispatcher<()> for NullDispatcher {
        async fn resolve<'a>(
            &'a self,
            _context: &'a (),
            _field_name: &'a str,
            _arguments: &'a JsonMap,
        ) -> Result<JsonValue, ResolverError> {
            Ok(JsonValue::Null)
        }
    }

    impl super::Dispatcher<()> for ValueDispatcher {
        async fn resolve<'a>(
            &'a self,
            _context: &'a (),
            _field_name: &'a str,
            _arguments: &'a JsonMap,
        ) -> Result<JsonValue, ResolverError> {
            Ok(self.0.clone())
        }
    }

    impl super::Dispatcher<()> for CountingDispatcher {
        async fn resolve<'a>(
            &'a self,
            _context: &'a (),
            _field_name: &'a str,
            _arguments: &'a JsonMap,
        ) -> Result<JsonValue, ResolverError> {
            self.0.fetch_add(1, Ordering::Relaxed);

            Ok(json!("unexpected resolver call"))
        }
    }

    impl super::Dispatcher<()> for RecoveringDispatcher {
        async fn resolve<'a>(
            &'a self,
            _context: &'a (),
            _field_name: &'a str,
            _arguments: &'a JsonMap,
        ) -> Result<JsonValue, ResolverError> {
            if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
                Err(ResolverError::new("Greeting failed."))
            } else {
                Ok(json!("Hello"))
            }
        }
    }

    fn assert_send<T: Send>(_: &T) {}

    fn coerce_arguments(schema_source: &str, request: Request) -> JsonMap {
        let schema = Schema::parse_and_validate(schema_source, "schema.graphql").unwrap();
        let prepared = prepare_request(&schema, &request).unwrap();
        let field = super::collect_fields(&schema, &prepared)
            .into_values()
            .next()
            .unwrap()[0];

        super::coerce_argument_values(&schema, &prepared, &[], field).unwrap()
    }

    async fn execute_value(field_type: &str, value: JsonValue) -> JsonValue {
        let schema = Schema::parse_and_validate(
            format!("type Query {{ values: {field_type} }}"),
            "schema.graphql",
        )
        .unwrap();
        let request = Request::new("query { values }");
        let dispatcher = ValueDispatcher(value);

        to_value(super::execute(&schema, &request, &dispatcher, &()).await).unwrap()
    }

    async fn execute_list_with_null_item(field_type: &str) -> JsonValue {
        execute_value(field_type, json!(["A", null, "B"])).await
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

        let fields = super::collect_fields(&schema, &prepared);
        let collected = fields.get("greeting").unwrap();

        assert_eq!(fields.len(), 1);
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].name.as_str(), "hello");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn field_inside_named_fragment_is_executed() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();
        let request = Request::new(
            r#"
                query {
                    ...Greeting
                }

                fragment Greeting on Query {
                    hello(name: "Sheri")
                }
            "#,
        );
        let context = TestContext { greeting: "Hello" };

        let response = super::execute(&schema, &request, &TestDispatcher, &context).await;

        assert_eq!(
            to_value(response).unwrap(),
            json!({
                "data": {
                    "hello": "Hello, Sheri"
                }
            })
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn merged_fields_are_executed_once() {
        let schema =
            Schema::parse_and_validate("type Query { hello: String! }", "schema.graphql").unwrap();
        let request = Request::new(
            r#"
                query {
                    hello
                    ...Greeting
                }

                fragment Greeting on Query {
                    hello
                }
            "#,
        );
        let dispatcher = CountingDispatcher(AtomicUsize::new(0));

        let response = super::execute(&schema, &request, &dispatcher, &()).await;

        assert_eq!(
            to_value(response).unwrap(),
            json!({ "data": { "hello": "unexpected resolver call" } })
        );
        assert_eq!(dispatcher.0.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn skipped_field_is_not_executed() {
        let schema =
            Schema::parse_and_validate("type Query { hello: String! }", "schema.graphql").unwrap();
        let request = Request::new("query { hello @skip(if: true) }");
        let dispatcher = CountingDispatcher(AtomicUsize::new(0));

        let response = super::execute(&schema, &request, &dispatcher, &()).await;

        assert_eq!(to_value(response).unwrap(), json!({ "data": {} }));
        assert_eq!(dispatcher.0.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fragment_include_directive_uses_coerced_variable() {
        let schema =
            Schema::parse_and_validate("type Query { hello: String! }", "schema.graphql").unwrap();
        let variables = json!({ "include": false }).as_object().unwrap().clone();
        let request = Request::new(
            r#"
                query Greeting($include: Boolean!) {
                    ...GreetingFields @include(if: $include)
                }

                fragment GreetingFields on Query {
                    hello
                }
            "#,
        )
        .with_variables(variables);
        let dispatcher = CountingDispatcher(AtomicUsize::new(0));

        let response = super::execute(&schema, &request, &dispatcher, &()).await;

        assert_eq!(to_value(response).unwrap(), json!({ "data": {} }));
        assert_eq!(dispatcher.0.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn non_applicable_inline_fragment_is_not_executed() {
        let schema = Schema::parse_and_validate(
            r#"
                interface Node { id: ID! }
                type Query implements Node { id: ID! }
                type Other implements Node { id: ID!, secret: String! }
            "#,
            "schema.graphql",
        )
        .unwrap();
        let request = Request::new(
            r#"
                query {
                    ... on Node {
                        ... on Other {
                            secret
                        }
                    }
                }
            "#,
        );
        let dispatcher = CountingDispatcher(AtomicUsize::new(0));

        let response = super::execute(&schema, &request, &dispatcher, &()).await;

        assert_eq!(to_value(response).unwrap(), json!({ "data": {} }));
        assert_eq!(dispatcher.0.load(Ordering::Relaxed), 0);
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
        let fields = super::collect_fields(&schema, &prepared);
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
        let field = super::collect_fields(&schema, &prepared)
            .get("greeting")
            .unwrap()[0];
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
        let field = super::collect_fields(&schema, &prepared)
            .get("greeting")
            .unwrap()[0];
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

    #[tokio::test(flavor = "current_thread")]
    async fn overlapping_requests_keep_their_context_values() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();
        let request = Request::new(r#"query { hello(name: "Sheri") }"#);
        let first_context = TestContext { greeting: "Hello" };
        let second_context = TestContext {
            greeting: "Bonjour",
        };
        let dispatcher = TestDispatcher;

        let (first, second) = tokio::join!(
            super::execute(&schema, &request, &dispatcher, &first_context),
            super::execute(&schema, &request, &dispatcher, &second_context),
        );

        assert_eq!(
            to_value(first).unwrap(),
            json!({ "data": { "hello": "Hello, Sheri" } })
        );
        assert_eq!(
            to_value(second).unwrap(),
            json!({ "data": { "hello": "Bonjour, Sheri" } })
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execution_succeeds_after_a_domain_error() {
        let schema =
            Schema::parse_and_validate("type Query { hello: String! }", "schema.graphql").unwrap();
        let request = Request::new("query { hello }");
        let dispatcher = RecoveringDispatcher(AtomicUsize::new(0));

        let failed = to_value(super::execute(&schema, &request, &dispatcher, &()).await).unwrap();
        let succeeded =
            to_value(super::execute(&schema, &request, &dispatcher, &()).await).unwrap();

        assert_eq!(failed.get("data"), Some(&JsonValue::Null));
        assert!(
            failed
                .get("errors")
                .and_then(JsonValue::as_array)
                .is_some_and(|errors| !errors.is_empty())
        );
        assert_eq!(succeeded, json!({ "data": { "hello": "Hello" } }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn variable_input_reaches_dispatcher_and_produces_greeting() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();
        let variables = json!({ "name": "Sheri" }).as_object().unwrap().clone();
        let request = Request::new(
            r#"
                query Greeting($name: String!) {
                    hello(name: $name)
                }
            "#,
        )
        .with_variables(variables);
        let greeting = String::from("Hello");
        let context = TestContext {
            greeting: &greeting,
        };

        let response = super::execute(&schema, &request, &TestDispatcher, &context).await;

        assert_eq!(
            to_value(response).unwrap(),
            json!({
                "data": {
                    "hello": "Hello, Sheri"
                }
            })
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invalid_variable_inputs_do_not_invoke_dispatcher() {
        let schema = Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap();
        let cases = [
            ("missing", JsonMap::new()),
            ("null", json!({ "name": null }).as_object().unwrap().clone()),
            (
                "incompatible",
                json!({ "name": 42 }).as_object().unwrap().clone(),
            ),
        ];
        let dispatcher = CountingDispatcher(AtomicUsize::new(0));

        for (case, variables) in cases {
            let request = Request::new(
                r#"
                    query Greeting($name: String!) {
                        hello(name: $name)
                    }
                "#,
            )
            .with_variables(variables);

            let response = super::execute(&schema, &request, &dispatcher, &()).await;
            let response = to_value(response).unwrap();

            assert!(response.get("data").is_none(), "{case}");
            assert!(
                response
                    .get("errors")
                    .and_then(JsonValue::as_array)
                    .is_some_and(|errors| !errors.is_empty()),
                "{case}"
            );
            assert_eq!(dispatcher.0.load(Ordering::Relaxed), 0, "{case}");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolver_error_on_non_null_root_field_propagates_null_to_data() {
        let schema =
            Schema::parse_and_validate("type Query { hello: String! }", "schema.graphql").unwrap();
        let request = Request::new("query { hello }");

        let response = super::execute(&schema, &request, &FailingDispatcher, &()).await;
        let response = to_value(response).unwrap();

        assert_eq!(response.get("data"), Some(&JsonValue::Null));
        assert_eq!(response["errors"][0]["message"], "Greeting failed.");
        assert_eq!(response["errors"][0]["path"], json!(["hello"]));
        assert_eq!(
            response["errors"][0]["extensions"]["code"],
            "GREETING_FAILED"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn non_null_field_returning_null_adds_field_error() {
        let schema =
            Schema::parse_and_validate("type Query { hello: String! }", "schema.graphql").unwrap();
        let request = Request::new("query { hello }");

        let response = super::execute(&schema, &request, &NullDispatcher, &()).await;
        let response = to_value(response).unwrap();

        assert_eq!(response.get("data"), Some(&JsonValue::Null));

        let errors = response
            .get("errors")
            .and_then(JsonValue::as_array)
            .expect("a non-null field returning null must produce an error");

        assert_eq!(errors.len(), 1);
        assert!(
            errors[0]["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty())
        );
        assert_eq!(errors[0]["path"], json!(["hello"]));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn nullable_list_preserves_null_items() {
        let response = execute_list_with_null_item("[String]").await;

        assert_eq!(
            response,
            json!({
                "data": {
                    "values": ["A", null, "B"]
                }
            })
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn non_null_list_preserves_nullable_items() {
        let response = execute_list_with_null_item("[String]!").await;

        assert_eq!(
            response,
            json!({
                "data": {
                    "values": ["A", null, "B"]
                }
            })
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn null_non_null_item_nullifies_nullable_list() {
        let response = execute_list_with_null_item("[String!]").await;

        assert_eq!(response["data"]["values"], JsonValue::Null);
        assert_eq!(response["errors"][0]["path"], json!(["values", 1]));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn null_non_null_item_propagates_through_non_null_list() {
        let response = execute_list_with_null_item("[String!]!").await;

        assert_eq!(response["data"], JsonValue::Null);
        assert_eq!(response["errors"][0]["path"], json!(["values", 1]));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn nullable_list_returning_non_list_becomes_null_with_execution_error() {
        let response = execute_value("[String]", json!("not a list")).await;

        assert_eq!(response["data"]["values"], JsonValue::Null);
        assert!(
            response["errors"][0]["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty())
        );
        assert_eq!(response["errors"][0]["path"], json!(["values"]));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn non_null_list_returning_non_list_propagates_null_to_root() {
        let response = execute_value("[String]!", json!("not a list")).await;

        assert_eq!(response["data"], JsonValue::Null);
        assert!(
            response["errors"][0]["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty())
        );
        assert_eq!(response["errors"][0]["path"], json!(["values"]));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn string_field_returning_incompatible_value_becomes_execution_error() {
        let schema =
            Schema::parse_and_validate("type Query { hello: String }", "schema.graphql").unwrap();
        let request = Request::new("query { hello }");
        let dispatcher = ValueDispatcher(json!({ "unexpected": true }));

        let response = super::execute(&schema, &request, &dispatcher, &()).await;
        let response = to_value(response).unwrap();

        assert_eq!(response["data"]["hello"], JsonValue::Null);
        assert!(
            response["errors"][0]["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty())
        );
        assert_eq!(response["errors"][0]["path"], json!(["hello"]));
    }
}
