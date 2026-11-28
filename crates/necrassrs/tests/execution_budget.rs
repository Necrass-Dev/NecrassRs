//! Executed fields, completed list items, and generated introspection values are
//! charged to per-response budgets.

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
    node: Node
    items: [Int!]!
    nodes: [Node!]!
    stall: Int
}
type Mutation { touch: Int }
type Subscription { ticks: Node! }
type Node {
    left: Node
    right: Node
    value: Int
}
"#;

const FIELD_LIMIT: &str = "Field limit exceeded";
const LIST_ITEM_LIMIT: &str = "List item limit exceeded";

#[derive(Default)]
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

    fn node(&self) -> ResolvedValue<()> {
        ResolvedValue::Object(Box::new(NodeObject(self.calls.clone())))
    }
}

impl Dispatcher<()> for TestDispatcher {
    async fn resolve<'a>(
        &'a self,
        _: &'a (),
        coordinate: FieldCoordinate<'a>,
        _: &'a JsonMap,
    ) -> Result<ResolvedValue<()>, ResolverError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if coordinate.field == "stall" {
            futures::future::pending::<()>().await;
        }
        Ok(match coordinate.field {
            "node" => self.node(),
            "items" => ResolvedValue::Json((0..self.items).collect::<Vec<_>>().into()),
            "nodes" => ResolvedValue::List((0..self.items).map(|_| self.node()).collect()),
            _ => ResolvedValue::Json(0.into()),
        })
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
            (0..2).map(|_| SourceEvent::Value(self.node())),
        )))
    }
}

struct NodeObject(Arc<AtomicUsize>);

impl ResolvedObject<()> for NodeObject {
    fn type_name(&self) -> &'static str {
        "Node"
    }

    fn resolve<'a>(
        &'a self,
        _: &'a (),
        coordinate: FieldCoordinate<'a>,
        _: &'a JsonMap,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<ResolvedValue<()>, ResolverError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(match coordinate.field {
                "value" => ResolvedValue::Json(0.into()),
                _ => ResolvedValue::Object(Box::new(NodeObject(self.0.clone()))),
            })
        })
    }
}

fn schema() -> Valid<Schema> {
    Schema::parse_and_validate(SDL, "schema.graphql").unwrap()
}

fn execute(dispatcher: &TestDispatcher, document: &str, options: ExecutionOptions) -> Value {
    let schema = schema();
    let response = block_on(execute_with_options(
        &schema,
        &Request::new(document),
        dispatcher,
        &(),
        options,
    ));
    serde_json::to_value(response).unwrap()
}

fn assert_single_error(response: &Value, prefix: &str) {
    assert_eq!(response["data"], Value::Null, "{response}");
    let errors = response["errors"].as_array().expect("errors");
    assert_eq!(errors.len(), 1, "{response}");
    let message = errors[0]["message"].as_str().unwrap();
    assert!(message.starts_with(prefix), "{message}");
}

fn aliases(count: usize, selection: &str) -> String {
    (0..count)
        .map(|index| format!("a{index}: {selection}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A fragment chain that doubles the executed fields at every level.
fn fan_out_document(depth: usize) -> String {
    let mut document = String::from("{ node { ...f0 } }");
    for level in 0..depth {
        let next = if level + 1 < depth {
            format!("...f{}", level + 1)
        } else {
            "value".to_owned()
        };
        document.push_str(&format!(
            " fragment f{level} on Node {{ left {{ {next} }} right {{ {next} }} }}"
        ));
    }
    document
}

#[test]
fn fragment_fan_out_stops_at_the_field_limit() {
    let dispatcher = TestDispatcher::default();
    let response = execute(
        &dispatcher,
        &fan_out_document(20),
        ExecutionOptions::default(),
    );

    assert_single_error(&response, FIELD_LIMIT);
    assert!(dispatcher.calls() <= 50_000, "{} calls", dispatcher.calls());
}

#[test]
fn typename_aliases_are_charged_as_fields() {
    let dispatcher = TestDispatcher::new(10_000);
    let response = execute(
        &dispatcher,
        &format!("{{ nodes {{ {} }} }}", aliases(200, "__typename")),
        ExecutionOptions::default(),
    );

    assert_single_error(&response, FIELD_LIMIT);
}

#[test]
fn aliased_lists_stop_at_the_list_item_limit() {
    let dispatcher = TestDispatcher::new(10_000);
    let response = execute(
        &dispatcher,
        &format!("{{ {} }}", aliases(1_000, "items")),
        ExecutionOptions::default(),
    );

    assert_single_error(&response, LIST_ITEM_LIMIT);
}

#[test]
fn work_at_the_limits_completes() {
    let dispatcher = TestDispatcher::new(3);
    let options = ExecutionOptions {
        // \`nodes\`, then \`value\` on each of the three nodes.
        field_limit: 4,
        list_item_limit: 3,
        ..Default::default()
    };
    let response = execute(&dispatcher, "{ nodes { value } }", options);
    assert_eq!(
        response,
        json!({ "data": { "nodes": [{ "value": 0 }, { "value": 0 }, { "value": 0 }] } })
    );

    let over_fields = ExecutionOptions {
        field_limit: 3,
        ..options
    };
    let response = execute(&dispatcher, "{ nodes { value } }", over_fields);
    assert_single_error(&response, FIELD_LIMIT);

    let over_items = ExecutionOptions {
        list_item_limit: 2,
        ..options
    };
    let response = execute(&dispatcher, "{ nodes { value } }", over_items);
    assert_single_error(&response, LIST_ITEM_LIMIT);
}

#[test]
fn mutation_fields_beyond_the_limit_do_not_start() {
    let dispatcher = TestDispatcher::default();
    let response = execute(
        &dispatcher,
        &format!("mutation {{ {} }}", aliases(3, "touch")),
        ExecutionOptions {
            field_limit: 2,
            ..Default::default()
        },
    );

    assert_single_error(&response, FIELD_LIMIT);
    assert_eq!(dispatcher.calls(), 0);
}

#[test]
fn each_subscription_event_has_its_own_budget() {
    let schema = schema();
    let dispatcher = TestDispatcher::default();
    let options = ExecutionOptions {
        // \`ticks\` and \`value\` for each event.
        field_limit: 2,
        ..Default::default()
    };
    let request = Request::new("subscription { ticks { value } }");
    let Ok(responses) = block_on(subscribe_with_options(
        &schema,
        &request,
        &dispatcher,
        &(),
        options,
    )) else {
        panic!("subscription rejected");
    };
    let responses = block_on(responses.collect::<Vec<_>>());

    assert_eq!(responses.len(), 2);
    for response in responses {
        let response = serde_json::to_value(response.unwrap()).unwrap();
        assert_eq!(response, json!({ "data": { "ticks": { "value": 0 } } }));
    }
}

#[test]
fn subscription_events_beyond_the_limit_fail_alone() {
    let schema = schema();
    let dispatcher = TestDispatcher::default();
    let options = ExecutionOptions {
        field_limit: 2,
        ..Default::default()
    };
    let request = Request::new("subscription { ticks { a: value b: value } }");
    let Ok(responses) = block_on(subscribe_with_options(
        &schema,
        &request,
        &dispatcher,
        &(),
        options,
    )) else {
        panic!("subscription rejected");
    };
    let responses = block_on(responses.collect::<Vec<_>>());

    assert_eq!(responses.len(), 2);
    for response in responses {
        let response = serde_json::to_value(response.unwrap()).unwrap();
        assert_single_error(&response, FIELD_LIMIT);
    }
}

#[test]
fn schema_aliases_stop_at_the_introspection_node_limit() {
    let dispatcher = TestDispatcher::default();
    let selection = "__schema { types { name fields { name args { name } } } }";
    let response = execute(
        &dispatcher,
        &format!("{{ {} }}", aliases(1_000, selection)),
        ExecutionOptions {
            // Isolate the introspection limit from the document size limit.
            token_limit: usize::MAX,
            ..Default::default()
        },
    );

    assert!(response.get("data").is_none(), "{response}");
    let errors = response["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{response}");
    assert!(
        errors[0]["message"]
            .as_str()
            .unwrap()
            .starts_with("Response exceeds the limit of 100000 values"),
        "{response}"
    );
    assert_eq!(dispatcher.calls(), 0);
}

#[test]
fn full_introspection_fits_the_default_limit() {
    let dispatcher = TestDispatcher::default();
    let response = execute(
        &dispatcher,
        include_str!("fixtures/introspection_query.graphql"),
        ExecutionOptions::default(),
    );

    assert!(response.get("errors").is_none(), "{response}");
    assert!(response["data"]["__schema"]["types"].is_array());
}

#[test]
fn introspection_node_limit_is_configurable() {
    let dispatcher = TestDispatcher::default();
    let options = |introspection_node_limit| ExecutionOptions {
        introspection_node_limit,
        ..Default::default()
    };

    // \`__type\` and its \`name\`.
    let document = r#"{ __type(name: "Node") { name } }"#;
    let response = execute(&dispatcher, document, options(2));
    assert_eq!(
        response,
        json!({ "data": { "__type": { "name": "Node" } } })
    );

    let response = execute(&dispatcher, document, options(1));
    assert!(response.get("data").is_none(), "{response}");
    assert_eq!(
        response["errors"].as_array().unwrap().len(),
        1,
        "{response}"
    );
}

#[test]
fn root_fields_after_an_exceeded_limit_do_not_start() {
    let dispatcher = TestDispatcher::new(10);
    let response = execute(
        &dispatcher,
        "{ a: items b: node { value } }",
        ExecutionOptions {
            list_item_limit: 9,
            ..Default::default()
        },
    );

    assert_single_error(&response, LIST_ITEM_LIMIT);
    // Only `items` ran; `node` was polled after the limit was exceeded.
    assert_eq!(dispatcher.calls(), 1);
}

#[test]
fn sibling_fields_after_an_exceeded_limit_do_not_start() {
    let dispatcher = TestDispatcher::default();
    let response = execute(
        &dispatcher,
        "{ node { left { value } right { value } } }",
        ExecutionOptions {
            // `node`, `left`, and `right`; `left.value` exceeds the limit.
            field_limit: 3,
            ..Default::default()
        },
    );

    assert_single_error(&response, FIELD_LIMIT);
    // `node` and `left` ran; `right` was polled after the limit was exceeded.
    assert_eq!(dispatcher.calls(), 2);
}

#[test]
fn exceeded_limits_do_not_wait_for_pending_resolvers() {
    use futures::FutureExt;

    let schema = schema();
    let dispatcher = TestDispatcher::new(10);
    let request = Request::new("{ a: stall b: items }");
    let response = execute_with_options(
        &schema,
        &request,
        &dispatcher,
        &(),
        ExecutionOptions {
            list_item_limit: 9,
            ..Default::default()
        },
    )
    .now_or_never()
    .expect("the error response must not wait for the stalled resolver");

    assert_single_error(&serde_json::to_value(response).unwrap(), LIST_ITEM_LIMIT);
}
