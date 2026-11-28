//! Execution stops once the response would exceed its node limit.

use reed_compiler::introspection;
use reed_compiler::resolvers::Execution;
use reed_compiler::resolvers::FieldError;
use reed_compiler::resolvers::ObjectValue;
use reed_compiler::resolvers::ResolveInfo;
use reed_compiler::resolvers::ResolvedValue;
use reed_compiler::response::JsonMap;
use reed_compiler::ExecutableDocument;
use reed_compiler::Schema;

const SDL: &str = "type Query { ok: Int failing: Int list: [Int] }";

struct Root;

impl ObjectValue for Root {
    fn type_name(&self) -> &str {
        "Query"
    }

    fn resolve_field<'a>(
        &'a self,
        info: &'a ResolveInfo<'a>,
    ) -> Result<ResolvedValue<'a>, FieldError> {
        match info.field_name() {
            "ok" => Ok(ResolvedValue::leaf(1)),
            "list" => Ok(ResolvedValue::list((0..3).map(ResolvedValue::leaf))),
            "failing" => Err(FieldError {
                message: "failed".into(),
            }),
            _ => Err(self.unknown_field_error(info)),
        }
    }
}

fn execute(query: &str, limit: usize) -> Result<serde_json::Value, String> {
    let schema = Schema::parse_and_validate(SDL, "schema.graphql").unwrap();
    let document = ExecutableDocument::parse_and_validate(&schema, query, "query.graphql").unwrap();
    Execution::new(&schema, &document)
        .response_node_limit(limit)
        .execute_sync(&Root)
        .map(|response| serde_json::to_value(response).unwrap())
        .map_err(|error| error.message().to_string())
}

#[test]
fn values_at_the_limit_complete() {
    // \`list\` and its three items.
    let response = execute("{ list }", 4).unwrap();
    assert_eq!(response["data"], serde_json::json!({ "list": [0, 1, 2] }));

    let error = execute("{ list }", 3).unwrap_err();
    assert_eq!(error, "Response exceeds the limit of 3 values.");
}

#[test]
fn nulls_from_resolver_errors_are_charged() {
    let response = execute("{ failing }", 1).unwrap();
    assert_eq!(response["data"], serde_json::json!({ "failing": null }));

    assert!(execute("{ failing }", 0).is_err());
    assert!(execute("{ ok failing }", 1).is_err());
}

#[test]
fn aliased_schema_introspection_stops_at_the_limit() {
    let schema = Schema::parse_and_validate(SDL, "schema.graphql").unwrap();
    let selection = "__schema { types { name fields { name } } }";
    let query = format!(
        "{{ {} }}",
        (0..100)
            .map(|index| format!("a{index}: {selection}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let document = ExecutableDocument::parse_and_validate(&schema, query, "query.graphql").unwrap();
    let operation = document.operations.get(None).unwrap();
    let variables = reed_compiler::validation::Valid::assume_valid(JsonMap::new());
    let execute = |limit| {
        introspection::partial_execute_with_node_limit(
            &schema,
            &schema.implementers_map(),
            &document,
            operation,
            &variables,
            limit,
        )
    };

    assert!(execute(usize::MAX).is_ok());
    assert!(execute(1_000).is_err());
}

#[test]
fn nulls_from_argument_errors_are_charged() {
    let sdl = "type Query { f(x: Int! = 1): Int }";
    let schema = Schema::parse_and_validate(sdl, "schema.graphql").unwrap();
    let document =
        ExecutableDocument::parse_and_validate(&schema, "query($v: Int) { f(x: $v) }", "q.graphql")
            .unwrap();
    let variables: JsonMap = serde_json::from_value(serde_json::json!({ "v": null })).unwrap();
    let variables = reed_compiler::validation::Valid::assume_valid(variables);
    let execute = |limit| {
        Execution::new(&schema, &document)
            .coerced_variable_values(&variables)
            .response_node_limit(limit)
            .execute_sync(&Root)
    };

    let response = serde_json::to_value(execute(1).unwrap()).unwrap();
    assert_eq!(
        response["data"],
        serde_json::json!({ "f": null }),
        "{response}"
    );
    assert!(execute(0).is_err());
}

#[test]
fn lists_stop_before_waiting_for_the_next_item() {
    use futures::future::BoxFuture;
    use futures::FutureExt;
    use futures::StreamExt;
    use reed_compiler::resolvers::AsyncObjectValue;
    use reed_compiler::resolvers::AsyncResolvedValue;

    struct AsyncRoot;

    impl AsyncObjectValue for AsyncRoot {
        fn type_name(&self) -> &str {
            "Query"
        }

        fn resolve_field<'a>(
            &'a self,
            _: &'a ResolveInfo<'a>,
        ) -> BoxFuture<'a, Result<AsyncResolvedValue<'a>, FieldError>> {
            Box::pin(async {
                // One ready item, then a stream that never yields again.
                let items = futures::stream::iter([Ok(AsyncResolvedValue::leaf(0))])
                    .chain(futures::stream::pending());
                Ok(AsyncResolvedValue::List(items.boxed()))
            })
        }
    }

    let schema = Schema::parse_and_validate(SDL, "schema.graphql").unwrap();
    let document =
        ExecutableDocument::parse_and_validate(&schema, "{ list }", "q.graphql").unwrap();
    // \`list\` fits; its first item exceeds the limit.
    let execution = Execution::new(&schema, &document).response_node_limit(1);
    let result = execution
        .execute_async(&AsyncRoot)
        .now_or_never()
        .expect("execution must not wait for the next list item");
    assert!(result.is_err());
}
