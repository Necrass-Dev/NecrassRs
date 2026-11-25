//! Resolver arguments are charged to a per-response argument work budget.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use futures::{StreamExt, executor::block_on, stream};
use necrassrs::{
    Dispatcher, ExecutionOptions, FieldCoordinate, JsonMap, Request, ResolvedObject, ResolvedValue,
    ResolverError, ScalarLiteralMap, Schema, SourceEvent, SourceStream, Valid,
    execute_with_options, subscribe_with_options,
};
use serde_json::{Value, json};

const SDL: &str = r#"
type Query {
    f(x: [String!]): Int
    g(x: [String!]): Int!
    items: [Item!]!
}
type Item {
    f(x: [String!]): Int
    h(x: [String!]!): Int
}
type Subscription { ticks: Item! }
"#;

const LIMIT_EXCEEDED: &str = "Argument work limit exceeded";

struct TestDispatcher {
    items: usize,
    calls: Arc<AtomicUsize>,
}

impl TestDispatcher {
    fn new(items: usize) -> Self {
        Self {
            items,
            calls: Arc::default(),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn item(&self) -> ResolvedValue<()> {
        ResolvedValue::Object(Box::new(Item(self.calls.clone())))
    }
}

impl Dispatcher<()> for TestDispatcher {
    async fn resolve<'a>(
        &'a self,
        _: &'a (),
        coordinate: FieldCoordinate<'a>,
        _: &'a JsonMap,
    ) -> Result<ResolvedValue<()>, ResolverError> {
        if coordinate.field == "items" {
            return Ok(ResolvedValue::List(
                (0..self.items).map(|_| self.item()).collect(),
            ));
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ResolvedValue::Json(0.into()))
    }

    async fn resolve_event_stream<'stream, 'call>(
        &'stream self,
        _: &'stream (),
        _: FieldCoordinate<'call>,
        _: &'call JsonMap,
        _: &'call ScalarLiteralMap,
    ) -> Result<SourceStream<'stream, ResolvedValue<()>>, ResolverError>
    where
        'stream: 'call,
    {
        Ok(SourceStream::from_stream(stream::iter(
            (0..2).map(|_| SourceEvent::Value(self.item())),
        )))
    }
}

struct Item(Arc<AtomicUsize>);

impl ResolvedObject<()> for Item {
    fn type_name(&self) -> &'static str {
        "Item"
    }

    fn resolve<'a>(
        &'a self,
        _: &'a (),
        _: FieldCoordinate<'a>,
        _: &'a JsonMap,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<ResolvedValue<()>, ResolverError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ResolvedValue::Json(0.into()))
        })
    }
}

fn schema() -> Valid<Schema> {
    Schema::parse_and_validate(SDL, "schema.graphql").unwrap()
}

fn options(argument_work_limit: usize) -> ExecutionOptions {
    ExecutionOptions {
        argument_work_limit,
        ..Default::default()
    }
}

/// A three-item list variable: four argument values per use.
fn list_variable() -> JsonMap {
    serde_json::from_value(json!({ "v": ["a", "b", "c"] })).unwrap()
}

fn execute(
    dispatcher: &TestDispatcher,
    document: &str,
    variables: JsonMap,
    options: ExecutionOptions,
) -> Value {
    let schema = schema();
    let request = Request::new(document).with_variables(variables);
    let response = block_on(execute_with_options(
        &schema,
        &request,
        dispatcher,
        &(),
        options,
    ));
    serde_json::to_value(response).unwrap()
}

fn assert_limit_errors(response: &Value, paths: &[Value]) {
    let errors = response["errors"].as_array().expect("errors");
    assert_eq!(
        errors
            .iter()
            .map(|error| &error["path"])
            .collect::<Vec<_>>(),
        paths.iter().collect::<Vec<_>>(),
        "{response}"
    );
    for error in errors {
        let message = error["message"].as_str().unwrap();
        assert!(message.starts_with(LIMIT_EXCEEDED), "{message}");
        assert!(error["locations"].is_array(), "{error}");
    }
}

#[test]
fn aliases_beyond_the_limit_get_field_errors() {
    let dispatcher = TestDispatcher::new(0);
    let response = execute(
        &dispatcher,
        "query($v: [String!]) { a: f(x: $v) b: f(x: $v) c: f(x: $v) }",
        list_variable(),
        options(8),
    );

    assert_eq!(response["data"], json!({ "a": 0, "b": 0, "c": null }));
    assert_limit_errors(&response, &[json!(["c"])]);
    assert_eq!(dispatcher.calls(), 2);
}

#[test]
fn literal_arguments_are_charged() {
    let dispatcher = TestDispatcher::new(0);
    let response = execute(
        &dispatcher,
        r#"{ a: f(x: ["a", "b", "c"]) b: f(x: ["a", "b", "c"]) c: f(x: ["a", "b", "c"]) }"#,
        JsonMap::new(),
        options(8),
    );

    assert_eq!(response["data"], json!({ "a": 0, "b": 0, "c": null }));
    assert_limit_errors(&response, &[json!(["c"])]);
}

#[test]
fn limit_errors_on_non_null_fields_propagate_null() {
    let dispatcher = TestDispatcher::new(0);
    let response = execute(
        &dispatcher,
        "query($v: [String!]) { a: g(x: $v) b: g(x: $v) c: g(x: $v) }",
        list_variable(),
        options(8),
    );

    assert_eq!(response["data"], Value::Null, "{response}");
    assert_limit_errors(&response, &[json!(["c"])]);
}

#[test]
fn each_list_item_is_charged() {
    let dispatcher = TestDispatcher::new(3);
    let response = execute(
        &dispatcher,
        "query($v: [String!]) { items { f(x: $v) } }",
        list_variable(),
        options(8),
    );

    assert_eq!(
        response["data"],
        json!({ "items": [{ "f": 0 }, { "f": 0 }, { "f": null }] })
    );
    assert_limit_errors(&response, &[json!(["items", 2, "f"])]);
    assert_eq!(dispatcher.calls(), 2);
}

#[test]
fn default_limit_bounds_list_item_amplification() {
    let dispatcher = TestDispatcher::new(10_000);
    let strings = (0..100_000).map(|index| json!(index.to_string()));
    let variables = serde_json::from_value(json!({ "v": strings.collect::<Vec<_>>() })).unwrap();
    let response = execute(
        &dispatcher,
        "query($v: [String!]) { items { f(x: $v) } }",
        variables,
        ExecutionOptions::default(),
    );

    // Each call passes 100,001 values, so the default limit of 1,000,000 admits nine calls.
    assert_eq!(dispatcher.calls(), 9);
    let errors = response["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 10_000 - 9);
    assert_eq!(errors[0]["path"], json!(["items", 9, "f"]));
}

#[test]
fn argument_errors_keep_each_item_path() {
    let dispatcher = TestDispatcher::new(2);
    let response = execute(
        &dispatcher,
        r#"query($v: [String!] = ["a"]) { items { h(x: $v) } }"#,
        serde_json::from_value(json!({ "v": null })).unwrap(),
        ExecutionOptions::default(),
    );

    assert_eq!(
        response["data"],
        json!({ "items": [{ "h": null }, { "h": null }] })
    );
    let errors = response["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2, "{response}");
    for (index, error) in errors.iter().enumerate() {
        assert_eq!(error["message"], "null value for non-null argument 'x'");
        assert_eq!(error["path"], json!(["items", index, "h"]));
    }
    assert_eq!(dispatcher.calls(), 0);
}

#[test]
fn zero_limit_allows_only_fields_without_argument_values() {
    let dispatcher = TestDispatcher::new(0);
    let response = execute(
        &dispatcher,
        "{ a: f b: f(x: []) }",
        JsonMap::new(),
        options(0),
    );

    assert_eq!(response["data"], json!({ "a": 0, "b": null }));
    assert_limit_errors(&response, &[json!(["b"])]);
}

#[test]
fn each_subscription_event_has_its_own_budget() {
    let schema = schema();
    let dispatcher = TestDispatcher::new(0);
    let request = Request::new("subscription($v: [String!]) { ticks { a: f(x: $v) b: f(x: $v) } }")
        .with_variables(list_variable());
    let Ok(responses) = block_on(subscribe_with_options(
        &schema,
        &request,
        &dispatcher,
        &(),
        options(8),
    )) else {
        panic!("subscription rejected");
    };
    let responses = block_on(responses.collect::<Vec<_>>());

    assert_eq!(responses.len(), 2);
    for response in responses {
        let response = serde_json::to_value(response.unwrap()).unwrap();
        assert_eq!(response, json!({ "data": { "ticks": { "a": 0, "b": 0 } } }));
    }
    assert_eq!(dispatcher.calls(), 4);
}

#[test]
fn rejected_aliases_do_not_copy_their_variables() {
    let strings = (0..100_000).map(|index| json!(index.to_string()));
    let variables: JsonMap =
        serde_json::from_value(json!({ "v": strings.collect::<Vec<_>>() })).unwrap();
    let elapsed = |aliases: usize| {
        let dispatcher = TestDispatcher::new(0);
        let fields = (0..aliases)
            .map(|index| format!("a{index}: f(x: $v)"))
            .collect::<Vec<_>>()
            .join(" ");
        let start = std::time::Instant::now();
        let response = execute(
            &dispatcher,
            &format!("query($v: [String!]) {{ {fields} }}"),
            variables.clone(),
            ExecutionOptions::default(),
        );
        let elapsed = start.elapsed();
        assert_eq!(dispatcher.calls(), 9);
        assert_eq!(response["errors"].as_array().unwrap().len(), aliases - 9);
        elapsed
    };

    let small = elapsed(300);
    let large = elapsed(1_200);
    assert!(
        large < small * 2,
        "rejecting 4x more aliases took {large:?} versus {small:?}"
    );
}
