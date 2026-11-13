use std::{
    cell::Cell,
    collections::VecDeque,
    future::Future,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context as TaskContext, Poll},
};

use futures::{StreamExt, executor::block_on, future, stream, task::noop_waker};
use necrassrs::{
    Dispatcher, ExecutionOptions, FieldCoordinate, JsonMap, Request, ResolvedObject, ResolvedValue,
    ResolverError, ResponseStream, ScalarLiteralMap, Schema, SourceEvent, SourceStream,
    SubscriptionError, Valid, subscribe, subscribe_with_options,
};
use serde_json::{Value, json};

const SDL: &str = r#"
type Query { ping: Int! }
type Subscription {
    events(id: ID!): Event!
    nullable: Event
    scalar: Int!
    rows: [Event]
    strictRows: [Event!]!
}
type Event { value: Int! left: Int! right: Int! label: String! fail: String! }
"#;

#[derive(Default)]
struct Context {
    label: String,
    creates: AtomicUsize,
    polls: AtomicUsize,
    source_drops: AtomicUsize,
    creation_drops: AtomicUsize,
    field_starts: AtomicUsize,
    field_drops: AtomicUsize,
    release: AtomicUsize,
    block_fields: bool,
}

struct DropCount<'a>(&'a AtomicUsize);

impl Drop for DropCount<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn count(value: &AtomicUsize) -> usize {
    value.load(Ordering::SeqCst)
}

fn send<T: Send>(value: T) -> T {
    value
}
fn unpin<T: Unpin>(_: &T) {}

fn tracked_source<'a, T: Send + 'a>(
    context: &'a Context,
    events: Vec<SourceEvent<T>>,
) -> SourceStream<'a, T> {
    let guard = DropCount(&context.source_drops);
    let mut events = VecDeque::from(events);
    let mut finished = false;
    SourceStream::from_stream(stream::poll_fn(move |_| {
        let _keep_alive = &guard;
        assert!(
            !finished,
            "source polled after terminal failure or exhaustion"
        );
        context.polls.fetch_add(1, Ordering::SeqCst);
        let next = events.pop_front();
        finished = matches!(&next, None | Some(SourceEvent::SourceError(_)));
        Poll::Ready(next)
    }))
}

struct TestDispatcher {
    events: Mutex<Option<Vec<SourceEvent<ResolvedValue<Context>>>>>,
    fail_creation: bool,
    pending_creation: bool,
    pending_source: bool,
}

impl TestDispatcher {
    fn new(events: Vec<SourceEvent<ResolvedValue<Context>>>) -> Self {
        Self {
            events: Mutex::new(Some(events)),
            fail_creation: false,
            pending_creation: false,
            pending_source: false,
        }
    }
}

impl Dispatcher<Context> for TestDispatcher {
    async fn resolve<'a>(
        &'a self,
        _context: &'a Context,
        coordinate: FieldCoordinate<'a>,
        _arguments: &'a JsonMap,
    ) -> Result<ResolvedValue<Context>, ResolverError> {
        assert_eq!(
            coordinate.parent_type, "Query",
            "subscription called ordinary root resolve"
        );
        Ok(ResolvedValue::Json(42.into()))
    }

    async fn resolve_event_stream<'stream, 'call>(
        &'stream self,
        context: &'stream Context,
        coordinate: FieldCoordinate<'call>,
        arguments: &'call JsonMap,
        _argument_literals: &'call ScalarLiteralMap,
    ) -> Result<SourceStream<'stream, ResolvedValue<Context>>, ResolverError>
    where
        'stream: 'call,
    {
        context.creates.fetch_add(1, Ordering::SeqCst);
        let _creation = DropCount(&context.creation_drops);
        if self.pending_creation {
            future::pending::<()>().await;
        }
        if self.fail_creation {
            return Err(
                ResolverError::new("Source unavailable").with_extension("code", "SOURCE_START")
            );
        }
        if self.pending_source {
            let guard = DropCount(&context.source_drops);
            return Ok(SourceStream::from_stream(stream::poll_fn(move |_| {
                let _keep_alive = &guard;
                context.polls.fetch_add(1, Ordering::SeqCst);
                Poll::Pending
            })));
        }
        assert_eq!(coordinate.parent_type, "Subscription");
        if coordinate.field == "events" {
            assert_eq!(
                arguments.get("id").and_then(|value| value.as_str()),
                Some("42")
            );
        }
        let events = self
            .events
            .lock()
            .unwrap()
            .take()
            .expect("source created more than once");
        Ok(tracked_source(context, events))
    }
}

struct Event(i32);

impl ResolvedObject<Context> for Event {
    fn type_name(&self) -> &'static str {
        "Event"
    }

    fn resolve<'a>(
        &'a self,
        context: &'a Context,
        coordinate: FieldCoordinate<'a>,
        _arguments: &'a JsonMap,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<ResolvedValue<Context>, ResolverError>> + Send + 'a>,
    > {
        Box::pin(async move {
            context.field_starts.fetch_add(1, Ordering::SeqCst);
            let _field = DropCount(&context.field_drops);
            if context.block_fields {
                future::poll_fn(|_| {
                    if count(&context.release) == 0 {
                        Poll::Pending
                    } else {
                        Poll::Ready(())
                    }
                })
                .await;
            }
            match coordinate.field {
                "value" | "left" | "right" => Ok(ResolvedValue::Json(self.0.into())),
                "label" => Ok(ResolvedValue::Json(context.label.clone().into())),
                "fail" if self.0 == 1 => {
                    Err(ResolverError::new("Event failed").with_extension("code", "EVENT"))
                }
                "fail" => Ok(ResolvedValue::Json("recovered".into())),
                _ => panic!("unexpected selected field"),
            }
        })
    }
}

fn event(value: i32) -> SourceEvent<ResolvedValue<Context>> {
    SourceEvent::Value(ResolvedValue::Object(Box::new(Event(value))))
}

fn schema() -> Valid<Schema> {
    Schema::parse_and_validate(SDL, "subscription.graphql").unwrap()
}

fn start<'a>(
    schema: &'a Valid<Schema>,
    dispatcher: &'a TestDispatcher,
    context: &'a Context,
    document: &str,
) -> ResponseStream<'a> {
    let request = Request::new(document);
    let future = send(subscribe(schema, &request, dispatcher, context));
    match block_on(future) {
        Ok(stream) => {
            unpin(&stream);
            send(stream)
        }
        Err(response) => panic!(
            "unexpected start failure: {}",
            serde_json::to_value(response).unwrap()
        ),
    }
}

fn response(stream: &mut ResponseStream<'_>) -> Value {
    let response = block_on(stream.next())
        .expect("missing response")
        .expect("unexpected terminal error");
    assert!(!response.is_request_error());
    serde_json::to_value(response).unwrap()
}

fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    let waker = noop_waker();
    std::pin::Pin::new(future).poll(&mut TaskContext::from_waker(&waker))
}

#[test]
fn source_stream_accepts_borrowing_non_sync_non_unpin_sources() {
    let label = String::from("Sheri");
    let borrowed = label.as_str();
    let state = Cell::new(0);
    let input = stream::unfold(state, move |state| async move {
        future::ready(()).await;
        if state.get() == 1 {
            return None;
        }
        state.set(1);
        Some((SourceEvent::Value(borrowed), state))
    });
    let mut source = send(SourceStream::from_stream(input));
    unpin(&source);
    assert!(matches!(
        block_on(source.next()),
        Some(SourceEvent::Value("Sheri"))
    ));
    assert!(block_on(source.next()).is_none());
    assert!(block_on(source.next()).is_none());
}

#[test]
fn source_stream_continues_after_field_error_but_stops_and_releases_on_source_error() {
    let context = Context::default();
    let mut source = tracked_source(
        &context,
        vec![
            SourceEvent::FieldError(ResolverError::new("event")),
            SourceEvent::Value(7),
            SourceEvent::SourceError(ResolverError::new("source")),
            SourceEvent::Value(99),
        ],
    );
    assert!(matches!(
        block_on(source.next()),
        Some(SourceEvent::FieldError(_))
    ));
    assert!(matches!(
        block_on(source.next()),
        Some(SourceEvent::Value(7))
    ));
    assert!(matches!(
        block_on(source.next()),
        Some(SourceEvent::SourceError(_))
    ));
    assert_eq!(
        count(&context.source_drops),
        1,
        "release before another poll or drop"
    );
    assert!(block_on(source.next()).is_none());
    assert!(block_on(source.next()).is_none());
    assert_eq!(count(&context.polls), 3);
}

#[test]
fn creates_before_return_and_reuses_prepared_request_with_merged_aliased_selections() {
    let schema = schema();
    let context = Context {
        label: "Sheri".into(),
        ..Context::default()
    };
    let dispatcher = TestDispatcher::new(vec![event(1), event(2)]);
    let mut responses = start(
        &schema,
        &dispatcher,
        &context,
        "subscription { changed: events(id: 42) { value } ...More } fragment More on Subscription { changed: events(id: 42) { label value } }",
    );
    assert_eq!(count(&context.creates), 1);
    assert_eq!(
        count(&context.polls),
        0,
        "creating a source must not consume its first event"
    );
    for value in [1, 2] {
        assert_eq!(
            response(&mut responses),
            json!({"data": {"changed": {"value": value, "label": "Sheri"}}})
        );
    }
    assert_eq!(
        count(&context.field_starts),
        4,
        "merged and unselected fields must not run twice"
    );
    assert!(block_on(responses.next()).is_none());
    assert_eq!(count(&context.source_drops), 1);
    assert!(block_on(responses.next()).is_none());
    assert_eq!(count(&context.polls), 3);
}

#[test]
fn invalid_requests_never_create_sources_or_execute_fields() {
    let schema = schema();
    let cases = [
        Request::new("subscription { events(id: 42) { value } scalar }"),
        Request::new("subscription { a: scalar b: scalar }"),
        Request::new("subscription { __typename }"),
        Request::new("subscription { scalar @skip(if: false) }"),
        Request::new(
            "subscription { ...Root @include(if: true) } fragment Root on Subscription { scalar }",
        ),
        Request::new("subscription { events { value } }"),
        Request::new("subscription($id: ID!) { events(id: $id) { value } }"),
        Request::new("subscription($id: ID!) { events(id: $id) { value } }")
            .with_variables(serde_json::from_value(json!({"id": null})).unwrap()),
        Request::new("subscription($id: ID!) { events(id: $id) { value } }")
            .with_variables(serde_json::from_value(json!({"id": {"invalid": true}})).unwrap()),
        Request::new("subscription A { scalar } subscription B { scalar }"),
        Request::new("subscription A { scalar }").with_operation_name("Missing"),
        Request::new("subscription {"),
    ];
    for request in cases {
        let context = Context::default();
        let dispatcher = TestDispatcher::new(vec![]);
        let result = block_on(subscribe(&schema, &request, &dispatcher, &context));
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("invalid request accepted"),
        };
        assert!(error.is_request_error());
        let json = serde_json::to_value(error).unwrap();
        assert!(json.get("data").is_none());
        assert!(!json["errors"].as_array().unwrap().is_empty());
        assert_eq!(count(&context.creates), 0);
        assert_eq!(count(&context.field_starts), 0);
    }
}

#[test]
fn source_creation_failure_is_a_request_error_without_data() {
    let schema = schema();
    let context = Context::default();
    let dispatcher = TestDispatcher {
        fail_creation: true,
        ..TestDispatcher::new(vec![])
    };
    let request = Request::new("subscription { scalar }");
    let result = block_on(subscribe(&schema, &request, &dispatcher, &context));
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("creation failure was deferred into the stream"),
    };
    assert!(error.is_request_error());
    let error = serde_json::to_value(error).unwrap();
    assert!(error.get("data").is_none());
    assert_eq!(error["errors"][0]["message"], "Source unavailable");
    assert_eq!(error["errors"][0]["extensions"]["code"], "SOURCE_START");
    assert_eq!(count(&context.creates), 1);
    assert_eq!(count(&context.creation_drops), 1);
    assert_eq!(count(&context.polls), 0);
}

#[test]
fn field_errors_and_root_null_are_results_and_do_not_poison_later_events() {
    let schema = schema();
    for nullable in [false, true] {
        let context = Context::default();
        let dispatcher = TestDispatcher::new(vec![
            SourceEvent::FieldError(
                ResolverError::new("Temporary").with_extension("code", "RETRY"),
            ),
            event(1),
            event(2),
        ]);
        let root = if nullable {
            "nullable"
        } else {
            "events(id: 42)"
        };
        let mut responses = start(
            &schema,
            &dispatcher,
            &context,
            &format!("subscription {{ changed: {root} {{ fail }} }}"),
        );
        let expected_data = if nullable {
            json!({"changed": null})
        } else {
            Value::Null
        };
        let field_error = response(&mut responses);
        assert_eq!(field_error["data"], expected_data);
        assert_eq!(field_error["errors"][0]["path"], json!(["changed"]));
        assert_eq!(field_error["errors"][0]["extensions"]["code"], "RETRY");
        assert_eq!(
            field_error["errors"][0]["locations"][0],
            json!({"line": 1, "column": 16})
        );
        assert_eq!(field_error["errors"].as_array().unwrap().len(), 1);
        let nested_error = response(&mut responses);
        assert_eq!(nested_error["data"], expected_data);
        assert_eq!(
            nested_error["errors"][0]["path"],
            json!(["changed", "fail"])
        );
        assert_eq!(nested_error["errors"].as_array().unwrap().len(), 1);
        assert_eq!(
            response(&mut responses),
            json!({"data": {"changed": {"fail": "recovered"}}})
        );
    }
}

#[test]
fn scalar_coercion_and_list_nullability_use_normal_completion_for_each_event() {
    let schema = schema();
    let context = Context::default();
    let dispatcher = TestDispatcher::new(vec![
        SourceEvent::Value(ResolvedValue::Json("not an Int".into())),
        SourceEvent::Value(ResolvedValue::Json(7.into())),
    ]);
    let mut responses = start(&schema, &dispatcher, &context, "subscription { scalar }");
    let error = response(&mut responses);
    assert_eq!(error["data"], Value::Null);
    assert_eq!(error["errors"][0]["path"], json!(["scalar"]));
    assert_eq!(response(&mut responses), json!({"data": {"scalar": 7}}));

    for root in ["rows", "strictRows"] {
        let context = Context::default();
        let dispatcher = TestDispatcher::new(vec![
            SourceEvent::Value(ResolvedValue::List(vec![
                ResolvedValue::Object(Box::new(Event(2))),
                ResolvedValue::Error(ResolverError::new("Item failed")),
            ])),
            SourceEvent::Value(ResolvedValue::List(vec![])),
        ]);
        let mut responses = start(
            &schema,
            &dispatcher,
            &context,
            &format!("subscription {{ items: {root} {{ value }} }}"),
        );
        let error = response(&mut responses);
        let expected = if root == "rows" {
            json!({"items": [{"value": 2}, null]})
        } else {
            Value::Null
        };
        assert_eq!(error["data"], expected);
        assert_eq!(error["errors"][0]["path"], json!(["items", 1]));
        assert_eq!(response(&mut responses), json!({"data": {"items": []}}));
    }
}

#[test]
fn terminal_source_failure_is_distinct_and_releases_without_another_poll() {
    let schema = schema();
    let context = Context::default();
    let dispatcher = TestDispatcher::new(vec![
        event(1),
        SourceEvent::SourceError(
            ResolverError::new("Closed").with_extension("code", "SOURCE_CLOSED"),
        ),
        event(2),
    ]);
    let mut responses = start(
        &schema,
        &dispatcher,
        &context,
        "subscription { events(id: 42) { value } }",
    );
    response(&mut responses);
    match block_on(responses.next()) {
        Some(Err(SubscriptionError::Source(error))) => {
            assert_eq!(error.message(), "Closed");
            assert_eq!(error.extensions().unwrap()["code"], "SOURCE_CLOSED");
        }
        _ => panic!("expected a terminal source error"),
    }
    assert_eq!(count(&context.source_drops), 1);
    assert!(block_on(responses.next()).is_none());
    assert!(block_on(responses.next()).is_none());
    assert_eq!(count(&context.polls), 2);
    fn standard_error<T: std::error::Error + Send + Sync>() {}
    standard_error::<SubscriptionError>();
    assert!(!SubscriptionError::Internal.to_string().is_empty());
}

#[test]
fn dropping_while_waiting_for_source_event_releases_source() {
    let schema = schema();
    let context = Context::default();
    let dispatcher = TestDispatcher {
        pending_source: true,
        ..TestDispatcher::new(vec![])
    };
    let mut responses = start(&schema, &dispatcher, &context, "subscription { scalar }");
    let mut next = Box::pin(responses.next());
    assert!(poll_once(&mut next).is_pending());
    assert_eq!(count(&context.polls), 1);
    assert_eq!(count(&context.field_starts), 0);
    drop(next);
    drop(responses);
    assert_eq!(count(&context.source_drops), 1);
}

#[test]
fn events_are_serial_but_sibling_fields_and_list_items_make_concurrent_progress() {
    let schema = schema();
    for list in [false, true] {
        let context = Context {
            block_fields: true,
            ..Context::default()
        };
        let events = if list {
            vec![SourceEvent::Value(ResolvedValue::List(vec![
                ResolvedValue::Object(Box::new(Event(1))),
                ResolvedValue::Object(Box::new(Event(2))),
            ]))]
        } else {
            vec![event(1), event(2)]
        };
        let dispatcher = TestDispatcher::new(events);
        let document = if list {
            "subscription { rows { value } }"
        } else {
            "subscription { events(id: 42) { left right } }"
        };
        let mut responses = start(&schema, &dispatcher, &context, document);
        let mut next = Box::pin(responses.next());
        assert!(poll_once(&mut next).is_pending());
        assert_eq!(
            count(&context.field_starts),
            2,
            "all independent fields/items must start before release"
        );
        assert_eq!(
            count(&context.polls),
            1,
            "next event must not be read while completing this event"
        );
        context.release.store(1, Ordering::SeqCst);
        let Poll::Ready(Some(Ok(first))) = poll_once(&mut next) else {
            panic!("released event did not complete")
        };
        drop(next);
        let first = serde_json::to_string(&first).unwrap();
        if list {
            assert_eq!(first, r#"{"data":{"rows":[{"value":1},{"value":2}]}}"#);
        } else {
            assert_eq!(first, r#"{"data":{"events":{"left":1,"right":1}}}"#);
            assert_eq!(
                response(&mut responses),
                json!({"data": {"events": {"left": 2, "right": 2}}})
            );
        }
    }
}

#[test]
fn dropping_response_drops_source_and_every_started_field_future() {
    let schema = schema();
    let context = Context {
        block_fields: true,
        ..Context::default()
    };
    let dispatcher = TestDispatcher::new(vec![event(1), event(2)]);
    let mut responses = start(
        &schema,
        &dispatcher,
        &context,
        "subscription { events(id: 42) { left right } }",
    );
    let mut next = Box::pin(responses.next());
    assert!(poll_once(&mut next).is_pending());
    assert_eq!(count(&context.field_starts), 2);
    assert_eq!(count(&context.field_drops), 0);
    drop(next);
    drop(responses);
    assert_eq!(count(&context.field_drops), 2);
    assert_eq!(count(&context.source_drops), 1);
    assert_eq!(count(&context.polls), 1);
}

#[test]
fn dropping_before_first_event_and_during_source_creation_releases_resources() {
    let schema = schema();
    let context = Context::default();
    let dispatcher = TestDispatcher::new(vec![event(1)]);
    let responses = start(
        &schema,
        &dispatcher,
        &context,
        "subscription { events(id: 42) { value } }",
    );
    drop(responses);
    assert_eq!(count(&context.source_drops), 1);
    assert_eq!(count(&context.polls), 0);

    let context = Context::default();
    let dispatcher = TestDispatcher {
        pending_creation: true,
        ..TestDispatcher::new(vec![])
    };
    let request = Request::new("subscription { scalar }");
    let mut start = Box::pin(send(subscribe(&schema, &request, &dispatcher, &context)));
    assert!(poll_once(&mut start).is_pending());
    assert_eq!(count(&context.creates), 1);
    assert_eq!(count(&context.creation_drops), 0);
    drop(start);
    assert_eq!(count(&context.creation_drops), 1);
}

#[test]
fn subscriptions_make_independent_progress_and_keep_contexts_isolated() {
    let schema = schema();
    let slow = Context {
        label: "slow".into(),
        block_fields: true,
        ..Context::default()
    };
    let fast = Context {
        label: "fast".into(),
        ..Context::default()
    };
    let first = TestDispatcher::new(vec![event(1)]);
    let second = TestDispatcher::new(vec![event(2)]);
    let mut a = start(
        &schema,
        &first,
        &slow,
        "subscription { events(id: 42) { label } }",
    );
    let mut b = start(
        &schema,
        &second,
        &fast,
        "subscription { events(id: 42) { label } }",
    );
    let mut pending = Box::pin(a.next());
    assert!(poll_once(&mut pending).is_pending());
    let mut next = Box::pin(b.next());
    let Poll::Ready(Some(Ok(result))) = poll_once(&mut next) else {
        panic!("unrelated subscription blocked")
    };
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        json!({"data": {"events": {"label": "fast"}}})
    );
    slow.release.store(1, Ordering::SeqCst);
    let Poll::Ready(Some(Ok(result))) = poll_once(&mut pending) else {
        panic!("released subscription blocked")
    };
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        json!({"data": {"events": {"label": "slow"}}})
    );
}

#[test]
fn options_operation_selection_and_nested_directives_are_preserved() {
    let schema = schema();
    let context = Context::default();
    let dispatcher = TestDispatcher::new(vec![event(3)]);
    let request = Request::new("subscription A { scalar } subscription B($show: Boolean!) { events(id: 42) { __typename value @include(if: $show) fail @skip(if: true) } }")
        .with_operation_name("B")
        .with_variables(serde_json::from_value(json!({"show": true})).unwrap());
    let result = block_on(send(subscribe_with_options(
        &schema,
        &request,
        &dispatcher,
        &context,
        ExecutionOptions {
            introspection: false,
            ..Default::default()
        },
    )));
    let mut responses = match result {
        Ok(stream) => stream,
        Err(_) => panic!("valid selected operation rejected"),
    };
    drop(request);
    assert_eq!(
        response(&mut responses),
        json!({"data": {"events": {"__typename": "Event", "value": 3}}})
    );
    assert_eq!(count(&context.field_starts), 1);
}

#[test]
fn legacy_dispatchers_keep_query_execution_and_report_unsupported_subscriptions() {
    struct Legacy;
    impl Dispatcher<()> for Legacy {
        async fn resolve<'a>(
            &'a self,
            _: &'a (),
            _: FieldCoordinate<'a>,
            _: &'a JsonMap,
        ) -> Result<ResolvedValue<()>, ResolverError> {
            Ok(ResolvedValue::Json(42.into()))
        }
    }
    let schema = schema();
    let result = block_on(necrassrs::execute(
        &schema,
        &Request::new("{ ping }"),
        &Legacy,
        &(),
    ));
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        json!({"data": {"ping": 42}})
    );
    match block_on(subscribe(
        &schema,
        &Request::new("subscription { scalar }"),
        &Legacy,
        &(),
    )) {
        Err(response) => assert!(response.is_request_error()),
        Ok(_) => panic!("legacy dispatcher must report unsupported source creation"),
    }
}

#[test]
fn dispatcher_stream_outlives_temporary_argument_maps() {
    let context = Context::default();
    let dispatcher = TestDispatcher::new(vec![event(1)]);
    let mut source = {
        let name = String::from("events");
        let arguments = serde_json::from_value(json!({"id": "42"})).unwrap();
        let literals = ScalarLiteralMap::new();
        block_on(send(dispatcher.resolve_event_stream(
            &context,
            FieldCoordinate {
                parent_type: "Subscription",
                field: &name,
            },
            &arguments,
            &literals,
        )))
        .unwrap_or_else(|error| panic!("{}", error.message()))
    };
    assert!(matches!(
        block_on(source.next()),
        Some(SourceEvent::Value(_))
    ));
}
