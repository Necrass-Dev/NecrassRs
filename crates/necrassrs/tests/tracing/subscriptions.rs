use std::{sync::atomic::Ordering, time::SystemTime};

use futures::StreamExt;
use necrassrs::{Request, ResolvedValue, SourceEvent, SubscriptionError, subscribe};
use opentelemetry::{
    Value,
    trace::{SpanKind, Status},
};
use serde_json::json;
use tracing::Instrument;

use super::{
    fixture::{Context, Events, failure, node, schema},
    support::{Capture, attr, child_of, one, send},
};

#[tokio::test]
async fn subscription_lifetime_includes_synchronous_preparation_and_exports_events_before_exhaustion()
 {
    let capture = Capture::default();
    let context = Context::default();
    let mut creation_finished = SystemTime::UNIX_EPOCH;
    capture
        .run(async {
            let schema = schema();
            let request = Request::new("subscription Watch { events { child { value } } }");
            let dispatcher = Events::new(vec![
                SourceEvent::Value(node(1)),
                SourceEvent::Value(node(2)),
            ]);
            let parent = tracing::info_span!("http.subscription");
            let startup = parent.in_scope(|| subscribe(&schema, &request, &dispatcher, &context));
            creation_finished = SystemTime::now();
            let mut responses = send(
                startup
                    .instrument(parent.clone())
                    .await
                    .unwrap_or_else(|_| panic!("valid subscription rejected")),
            );
            drop(request);
            for value in [1, 2] {
                let response = responses
                    .next()
                    .instrument(tracing::info_span!("unrelated.poller"))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    serde_json::to_value(response).unwrap(),
                    json!({"data": {"events": {"child": {"value": value}}}})
                );
                let finished = capture.spans();
                assert!(
                    !finished.iter().any(|span| span.name == "subscription"),
                    "operation stays open between events"
                );
                assert_eq!(
                    finished
                        .iter()
                        .filter(|span| span.name == "necrassrs.subscription.event")
                        .count(),
                    value as usize
                );
            }
            assert!(responses.next().await.is_none());
            assert_eq!(context.source_drops.load(Ordering::SeqCst), 1);
            drop(responses);
            drop(parent);
        })
        .await;
    let spans = capture.spans();
    let operation = one(&spans, "subscription");
    assert_eq!(operation.span_kind, SpanKind::Server);
    assert_eq!(
        attr(operation, "graphql.operation.type"),
        Some(&Value::from("subscription"))
    );
    assert_eq!(
        attr(operation, "graphql.operation.name"),
        Some(&Value::from("Watch"))
    );
    assert_eq!(operation.status, Status::Unset);
    assert_eq!(attr(operation, "necrassrs.cancelled"), None);
    child_of(operation, one(&spans, "http.subscription"));
    for name in ["necrassrs.parse", "necrassrs.validate"] {
        let phase = one(&spans, name);
        child_of(phase, operation);
        assert!(
            phase.end_time <= creation_finished,
            "synchronous preparation is instrumented when subscribe is called"
        );
    }
    child_of(one(&spans, "necrassrs.coerce_variables"), operation);
    let events: Vec<_> = spans
        .iter()
        .filter(|span| span.name == "necrassrs.subscription.event")
        .collect();
    assert_eq!(events.len(), 2);
    for event in events {
        child_of(event, operation);
        assert_eq!(event.span_kind, SpanKind::Internal);
        assert_eq!(event.status, Status::Unset);
        let root = spans
            .iter()
            .find(|span| {
                span.name == "Subscription.events"
                    && span.parent_span_id == event.span_context.span_id()
            })
            .expect("each event has its own root field span");
        child_of(root, event);
        assert_eq!(
            attr(root, "necrassrs.graphql.field.path"),
            Some(&Value::from("events"))
        );
        let child = spans
            .iter()
            .find(|span| {
                span.name == "Node.child" && span.parent_span_id == root.span_context.span_id()
            })
            .unwrap();
        child_of(child, root);
    }
}

#[tokio::test]
async fn event_processing_duration_excludes_waiting_for_source() {
    let capture = Capture::default();
    let context = Context::default();
    let mut released_at = SystemTime::UNIX_EPOCH;
    capture
        .run(async {
            let schema = schema();
            let request = Request::new("subscription { scalar }");
            let dispatcher = Events {
                wait_source: true,
                ..Events::new(vec![SourceEvent::Value(ResolvedValue::Json(7.into()))])
            };
            let mut responses = subscribe(&schema, &request, &dispatcher, &context)
                .await
                .unwrap_or_else(|_| panic!("startup failed"));
            assert!(futures::poll!(responses.next()).is_pending());
            released_at = SystemTime::now();
            context.release.store(true, Ordering::SeqCst);
            let response = responses.next().await.unwrap().unwrap();
            assert_eq!(
                serde_json::to_value(response).unwrap(),
                json!({"data": {"scalar": 7}})
            );
            assert!(responses.next().await.is_none());
        })
        .await;
    let spans = capture.spans();
    let event = one(&spans, "necrassrs.subscription.event");
    assert!(
        event.start_time >= released_at,
        "source waiting must not inflate processing duration"
    );
    assert!(one(&spans, "subscription").start_time <= released_at);
    child_of(event, one(&spans, "subscription"));
    assert!(!spans.iter().any(|span| span.name == "Subscription.scalar"));
}

async fn startup_failure(document: &str, fail_start: bool) {
    let capture = Capture::default();
    let context = Context::default();
    capture
        .run(async {
            let schema = schema();
            let dispatcher = Events {
                fail_start,
                ..Events::new(vec![])
            };
            let result = subscribe(&schema, &Request::new(document), &dispatcher, &context).await;
            let Err(response) = result else {
                panic!("startup must fail")
            };
            assert!(response.is_request_error());
        })
        .await;
    let spans = capture.spans();
    let operation = one(
        &spans,
        if fail_start {
            "subscription"
        } else {
            "GraphQL Operation"
        },
    );
    assert_eq!(operation.span_kind, SpanKind::Server);
    assert!(matches!(operation.status, Status::Error { .. }));
    assert_eq!(
        attr(operation, "error.type"),
        Some(&Value::from("necrassrs.request_error"))
    );
    assert_eq!(attr(operation, "necrassrs.cancelled"), None);
    assert!(
        !spans
            .iter()
            .any(|span| span.name == "necrassrs.subscription.event")
    );
}

#[tokio::test]
async fn subscription_parse_failure_closes_operation_without_events() {
    startup_failure("subscription {", false).await;
}

#[tokio::test]
async fn source_creation_failure_closes_operation_without_events() {
    startup_failure("subscription { scalar }", true).await;
}

#[tokio::test]
async fn failed_event_marks_stream_but_does_not_poison_later_event_status() {
    let capture = Capture::default();
    capture
        .run(async {
            let schema = schema();
            let context = Context::default();
            let dispatcher = Events::new(vec![
                SourceEvent::FieldError(failure()),
                SourceEvent::Value(node(2)),
            ]);
            let mut responses = subscribe(
                &schema,
                &Request::new("subscription { events { value } }"),
                &dispatcher,
                &context,
            )
            .await
            .unwrap_or_else(|_| panic!("startup failed"));
            let failed = serde_json::to_value(responses.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(failed["data"], json!(null));
            assert_eq!(failed["errors"].as_array().unwrap().len(), 1);
            assert_eq!(
                serde_json::to_value(responses.next().await.unwrap().unwrap()).unwrap(),
                json!({"data": {"events": {"value": 2}}})
            );
            assert!(responses.next().await.is_none());
        })
        .await;
    let spans = capture.spans();
    let mut events: Vec<_> = spans
        .iter()
        .filter(|span| span.name == "necrassrs.subscription.event")
        .collect();
    events.sort_by_key(|span| span.start_time);
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0].status, Status::Error { .. }));
    assert_eq!(
        attr(events[0], "error.type"),
        Some(&Value::from("necrassrs.execution_error"))
    );
    assert_eq!(events[1].status, Status::Unset);
    assert_eq!(attr(events[1], "error.type"), None);
    let operation = one(&spans, "subscription");
    assert!(matches!(operation.status, Status::Error { .. }));
    assert_eq!(
        attr(operation, "error.type"),
        Some(&Value::from("necrassrs.execution_error"))
    );
    assert_eq!(
        spans.iter().map(|span| span.events.len()).sum::<usize>(),
        1,
        "do not repeat the event error on the overall subscription"
    );
}

#[tokio::test]
async fn terminal_source_error_closes_span_and_releases_source_immediately() {
    let capture = Capture::default();
    let context = Context::default();
    capture
        .run(async {
            let schema = schema();
            let dispatcher = Events::new(vec![
                SourceEvent::SourceError(failure()),
                SourceEvent::Value(node(9)),
            ]);
            let mut responses = subscribe(
                &schema,
                &Request::new("subscription { events { value } }"),
                &dispatcher,
                &context,
            )
            .await
            .unwrap_or_else(|_| panic!("startup failed"));
            assert!(matches!(
                responses.next().await,
                Some(Err(SubscriptionError::Source(_)))
            ));
            assert_eq!(context.source_drops.load(Ordering::SeqCst), 1);
            one(&capture.spans(), "subscription");
            assert!(responses.next().await.is_none());
            assert_eq!(context.source_polls.load(Ordering::SeqCst), 1);
        })
        .await;
    let spans = capture.spans();
    let operation = one(&spans, "subscription");
    assert!(matches!(operation.status, Status::Error { .. }));
    assert_eq!(
        attr(operation, "error.type"),
        Some(&Value::from("necrassrs.subscription_source_error"))
    );
    assert_eq!(attr(operation, "necrassrs.cancelled"), None);
    assert_eq!(spans.iter().map(|span| span.events.len()).sum::<usize>(), 1);
    assert!(!format!("{spans:?}").contains("error-message-secret"));
    assert!(!format!("{spans:?}").contains("extension-secret"));
}

async fn cancel(wait_start: bool, wait_source: bool, during_event: bool) {
    let capture = Capture::default();
    let context = Context {
        block: during_event,
        ..Default::default()
    };
    capture
        .run(async {
            let schema = schema();
            let dispatcher = Events {
                wait_start,
                wait_source,
                ..Events::new(vec![SourceEvent::Value(node(1))])
            };
            let request = Request::new("subscription { events { child { value } } }");
            if wait_start {
                let mut startup = Box::pin(subscribe(&schema, &request, &dispatcher, &context));
                assert!(futures::poll!(&mut startup).is_pending());
                drop(startup);
                assert_eq!(context.creation_drops.load(Ordering::SeqCst), 1);
            } else {
                let mut responses = subscribe(&schema, &request, &dispatcher, &context)
                    .await
                    .unwrap_or_else(|_| panic!("startup failed"));
                if wait_source || during_event {
                    assert!(futures::poll!(responses.next()).is_pending());
                }
                drop(responses);
                assert_eq!(context.source_drops.load(Ordering::SeqCst), 1);
            }
        })
        .await;
    if during_event {
        assert_eq!(context.drops.load(Ordering::SeqCst), 1);
    }
    let spans = capture.spans();
    let operation = one(&spans, "subscription");
    assert_eq!(
        attr(operation, "necrassrs.cancelled"),
        Some(&Value::Bool(true))
    );
    assert_eq!(operation.status, Status::Unset);
    assert_eq!(attr(operation, "error.type"), None);
    if during_event {
        child_of(one(&spans, "necrassrs.subscription.event"), operation);
        one(&spans, "Subscription.events");
        one(&spans, "Node.child");
    } else {
        assert!(
            !spans
                .iter()
                .any(|span| span.name == "necrassrs.subscription.event")
        );
    }
}

#[tokio::test]
async fn cancellation_during_source_creation_closes_span() {
    cancel(true, false, false).await;
}
#[tokio::test]
async fn cancellation_before_first_poll_closes_span() {
    cancel(false, false, false).await;
}
#[tokio::test]
async fn cancellation_while_waiting_closes_span() {
    cancel(false, true, false).await;
}
#[tokio::test]
async fn cancellation_during_event_drops_fields_and_closes_spans() {
    cancel(false, false, true).await;
}

#[tokio::test]
async fn nested_event_error_is_recorded_once_on_nearest_instrumented_field() {
    let capture = Capture::default();
    capture
        .run(async {
            let schema = schema();
            let context = Context::default();
            let dispatcher = Events::new(vec![SourceEvent::Value(node(1))]);
            let mut responses = subscribe(
                &schema,
                &Request::new("subscription { events { child { fatal } } }"),
                &dispatcher,
                &context,
            )
            .await
            .unwrap_or_else(|_| panic!("startup failed"));
            let response = serde_json::to_value(responses.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(response["data"], json!({"events": {"child": null}}));
            assert_eq!(response["errors"].as_array().unwrap().len(), 1);
            assert!(responses.next().await.is_none());
        })
        .await;
    let spans = capture.spans();
    let field = one(&spans, "Node.child");
    assert_eq!(field.events.len(), 1);
    assert!(
        field.events[0]
            .attributes
            .iter()
            .any(|kv| kv.key.as_str() == "necrassrs.graphql.field.path"
                && kv.value == Value::from("events.child.fatal"))
    );
    assert_eq!(spans.iter().map(|span| span.events.len()).sum::<usize>(), 1);
    for name in ["subscription", "necrassrs.subscription.event"] {
        let span = one(&spans, name);
        assert!(matches!(span.status, Status::Error { .. }));
        assert_eq!(
            attr(span, "error.type"),
            Some(&Value::from("necrassrs.execution_error"))
        );
    }
}

#[tokio::test]
async fn concurrent_subscriptions_preserve_context_and_progress_independently() {
    let capture = Capture::default();
    capture
        .run(async {
            let schema = schema();
            let slow_context = Context {
                block: true,
                ..Default::default()
            };
            let fast_context = Context::default();
            let slow = Events::new(vec![SourceEvent::Value(node(1))]);
            let fast = Events::new(vec![SourceEvent::Value(node(2))]);
            let first = Request::new("subscription First { events { child { value } } }");
            let second = Request::new("subscription Second { events { child { value } } }");
            let a_parent = tracing::info_span!("http.slow");
            let b_parent = tracing::info_span!("http.fast");
            let a_start = a_parent.in_scope(|| subscribe(&schema, &first, &slow, &slow_context));
            let b_start = b_parent.in_scope(|| subscribe(&schema, &second, &fast, &fast_context));
            let mut a = send(
                a_start
                    .instrument(a_parent.clone())
                    .await
                    .unwrap_or_else(|_| panic!("slow startup failed")),
            );
            let mut b = send(
                b_start
                    .instrument(b_parent.clone())
                    .await
                    .unwrap_or_else(|_| panic!("fast startup failed")),
            );
            assert!(futures::poll!(a.next()).is_pending());
            let fast_response = serde_json::to_value(b.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(fast_response["data"]["events"]["child"]["value"], 2);
            assert!(b.next().await.is_none());
            slow_context.release.store(true, Ordering::SeqCst);
            let slow_response = serde_json::to_value(a.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(slow_response["data"]["events"]["child"]["value"], 1);
            assert!(a.next().await.is_none());
        })
        .await;
    let spans = capture.spans();
    for (name, parent) in [("First", "http.slow"), ("Second", "http.fast")] {
        let parent = one(&spans, parent);
        let trace: Vec<_> = spans
            .iter()
            .filter(|span| span.span_context.trace_id() == parent.span_context.trace_id())
            .cloned()
            .collect();
        let operation = one(&trace, "subscription");
        assert_eq!(
            attr(operation, "graphql.operation.name"),
            Some(&Value::from(name))
        );
        child_of(operation, parent);
        child_of(one(&trace, "necrassrs.subscription.event"), operation);
        child_of(
            one(&trace, "Node.child"),
            one(&trace, "Subscription.events"),
        );
    }
    assert_ne!(
        one(&spans, "http.slow").span_context.trace_id(),
        one(&spans, "http.fast").span_context.trace_id()
    );
}
