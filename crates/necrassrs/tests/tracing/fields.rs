use std::sync::atomic::Ordering;

use necrassrs::{Request, execute};
use opentelemetry::{
    Value,
    trace::{SpanKind, Status},
};
use serde_json::json;

use super::{
    fixture::{Context, Query, schema},
    support::{Capture, attr, child_of, one},
};

#[tokio::test]
async fn default_fields_follow_named_types_merge_selections_and_preserve_paths() {
    let capture = Capture::default();
    let request = Request::new(
        r#"{
        ping custom state scalars enums
        alias: node { child { value } }
        alias: node { child { value } }
        rows: nodes { next: child { value } }
        named { child { value } }
        search { ... on Node { child { value } } }
        skipped: node @skip(if: true) { child { value } }
        excluded: node @include(if: false) { child { value } }
    }"#,
    );
    let response = capture
        .run(execute(&schema(), &request, &Query, &Context::default()))
        .await;
    let response = serde_json::to_value(response).unwrap();
    assert!(response.get("errors").is_none(), "{response}");
    assert_eq!(
        response["data"]["rows"],
        json!([{"next": {"value": 1}}, {"next": {"value": 2}}])
    );
    assert!(response["data"].get("skipped").is_none());
    let spans = capture.spans();
    let mut fields: Vec<_> = spans
        .iter()
        .filter_map(|span| {
            attr(span, "necrassrs.graphql.field.path")
                .map(|path| (span.name.to_string(), path.to_string()))
        })
        .collect();
    fields.sort();
    let mut expected: Vec<_> = [
        ("Query.node", "alias"),
        ("Node.child", "alias.child"),
        ("Query.nodes", "rows"),
        ("Node.child", "rows.0.next"),
        ("Node.child", "rows.1.next"),
        ("Query.named", "named"),
        ("Node.child", "named.child"),
        ("Query.search", "search"),
        ("Node.child", "search.0.child"),
        ("Node.child", "search.1.child"),
    ]
    .into_iter()
    .map(|(name, path)| (name.to_owned(), path.to_owned()))
    .collect();
    expected.sort();
    assert_eq!(
        fields, expected,
        "only eligible executed fields get spans; list items alone do not"
    );
    for span in spans
        .iter()
        .filter(|span| attr(span, "necrassrs.graphql.field.path").is_some())
    {
        assert_eq!(span.span_kind, SpanKind::Internal);
    }
    let execute = one(&spans, "necrassrs.execute");
    for name in ["Query.node", "Query.nodes", "Query.named", "Query.search"] {
        child_of(one(&spans, name), execute);
    }
    for child in spans.iter().filter(|span| span.name == "Node.child") {
        let path = attr(child, "necrassrs.graphql.field.path")
            .unwrap()
            .to_string();
        let parent = match path.split('.').next().unwrap() {
            "alias" => "Query.node",
            "rows" => "Query.nodes",
            "named" => "Query.named",
            "search" => "Query.search",
            _ => unreachable!(),
        };
        child_of(child, one(&spans, parent));
    }
}

#[tokio::test]
async fn field_duration_contains_resolver_and_descendant_completion() {
    let capture = Capture::default();
    let response = capture
        .run(execute(
            &schema(),
            &Request::new("{ node { child { value } } }"),
            &Query,
            &Context::default(),
        ))
        .await;
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({"data": {"node": {"child": {"value": 1}}}})
    );
    let spans = capture.spans();
    let root = one(&spans, "Query.node");
    let child = one(&spans, "Node.child");
    let resolver = one(&spans, "app.resolve");
    child_of(resolver, root);
    child_of(child, root);
    assert!(
        resolver.end_time <= child.start_time,
        "completion happens after the root resolver returns"
    );
    for probe in spans.iter().filter(|span| span.name == "app.child") {
        child_of(probe, child);
    }
}

#[tokio::test]
async fn field_instrumentation_has_no_separate_depth_limit() {
    let capture = Capture::default();
    let mut selection = "value".to_owned();
    for _ in 0..12 {
        selection = format!("child {{ {selection} }}");
    }
    let response = capture
        .run(execute(
            &schema(),
            &Request::new(format!("{{ node {{ {selection} }} }}")),
            &Query,
            &Context::default(),
        ))
        .await;
    assert!(
        serde_json::to_value(response)
            .unwrap()
            .get("errors")
            .is_none()
    );
    let spans = capture.spans();
    assert_eq!(
        spans
            .iter()
            .filter(|span| span.name == "Node.child")
            .count(),
        12
    );
}

async fn execution_failure(
    document: &str,
    expected_data: serde_json::Value,
    paths: &[&str],
    enclosing: &str,
) {
    let capture = Capture::default();
    let response = capture
        .run(execute(
            &schema(),
            &Request::new(document),
            &Query,
            &Context::default(),
        ))
        .await;
    assert!(!response.is_request_error());
    let response = serde_json::to_value(response).unwrap();
    assert_eq!(response["data"], expected_data);
    assert_eq!(response["errors"].as_array().unwrap().len(), paths.len());
    let spans = capture.spans();
    let operation = one(&spans, "query");
    assert!(matches!(operation.status, Status::Error { .. }));
    assert_eq!(
        attr(operation, "error.type"),
        Some(&Value::from("necrassrs.execution_error"))
    );
    let events: Vec<_> = spans
        .iter()
        .flat_map(|span| span.events.iter().map(move |event| (span, event)))
        .collect();
    assert_eq!(
        events.len(),
        paths.len(),
        "each error is recorded once, including after null propagation"
    );
    let mut recorded_paths = Vec::new();
    for (span, event) in events {
        assert_eq!(
            span.name, enclosing,
            "omitted fields use the nearest execution span"
        );
        recorded_paths.push(
            event
                .attributes
                .iter()
                .find(|kv| kv.key.as_str() == "necrassrs.graphql.field.path")
                .expect("error event needs its response path")
                .value
                .to_string(),
        );
        assert!(
            event
                .attributes
                .iter()
                .any(|kv| kv.key.as_str() == "error.type"),
            "error event needs its category"
        );
    }
    recorded_paths.sort();
    let mut expected: Vec<_> = paths.iter().map(|path| path.to_string()).collect();
    expected.sort();
    assert_eq!(recorded_paths, expected);
}

#[tokio::test]
async fn partial_success_records_errors_even_for_omitted_scalar_fields() {
    execution_failure(
        "{ ping failed: fail }",
        json!({"ping": 42, "failed": null}),
        &["failed"],
        "necrassrs.execute",
    )
    .await;
}

#[tokio::test]
async fn multiple_errors_keep_common_operation_classification_and_individual_events() {
    execution_failure(
        "{ a: fail b: bad }",
        json!({"a": null, "b": null}),
        &["a", "b"],
        "necrassrs.execute",
    )
    .await;
}

#[tokio::test]
async fn nested_non_null_propagation_does_not_duplicate_error_events() {
    execution_failure(
        "{ node { child { fatal } } }",
        json!({"node": {"child": null}}),
        &["node.child.fatal"],
        "Node.child",
    )
    .await;
}

#[tokio::test]
async fn root_non_null_propagation_sets_error_status_with_null_data() {
    execution_failure(
        "{ node { fatal } }",
        json!(null),
        &["node.fatal"],
        "Query.node",
    )
    .await;
}

#[tokio::test]
async fn list_conversion_error_has_index_path_without_an_item_span() {
    execution_failure(
        "{ broken { value } }",
        json!({"broken": [{"value": 1}, null]}),
        &["broken.1"],
        "Query.broken",
    )
    .await;
}

#[tokio::test]
async fn argument_coercion_failure_is_inside_field_span_before_resolver_invocation() {
    let capture = Capture::default();
    let context = Context::default();
    let request =
        Request::new("query($required: Int) { aliased: argument(required: $required) { value } }")
            .with_variables(serde_json::from_value(json!({"required": null})).unwrap());
    let response = capture
        .run(execute(&schema(), &request, &Query, &context))
        .await;
    assert!(!response.is_request_error());
    let response = serde_json::to_value(response).unwrap();
    assert_eq!(response["data"], json!({"aliased": null}));
    assert_eq!(response["errors"].as_array().unwrap().len(), 1);
    assert!(
        context.order.lock().unwrap().is_empty(),
        "invalid arguments never reach the resolver"
    );
    let spans = capture.spans();
    let field = one(&spans, "Query.argument");
    child_of(field, one(&spans, "necrassrs.execute"));
    assert_eq!(
        attr(field, "necrassrs.graphql.field.path"),
        Some(&Value::from("aliased"))
    );
    assert_eq!(field.events.len(), 1);
    assert!(
        field.start_time <= field.events[0].timestamp
            && field.events[0].timestamp <= field.end_time
    );
    assert_eq!(spans.iter().map(|span| span.events.len()).sum::<usize>(), 1);
    assert_eq!(
        attr(one(&spans, "query"), "error.type"),
        Some(&Value::from("necrassrs.execution_error"))
    );
}

#[tokio::test]
async fn resolver_and_completion_errors_have_distinguishable_event_categories() {
    let capture = Capture::default();
    let response = capture
        .run(execute(
            &schema(),
            &Request::new("{ fail bad }"),
            &Query,
            &Context::default(),
        ))
        .await;
    assert_eq!(
        serde_json::to_value(response).unwrap()["errors"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let spans = capture.spans();
    let events: Vec<_> = spans.iter().flat_map(|span| span.events.iter()).collect();
    assert_eq!(events.len(), 2);
    let categories: Vec<_> = events
        .iter()
        .map(|event| {
            event
                .attributes
                .iter()
                .find(|kv| kv.key.as_str() == "error.type")
                .expect("each event needs an error category")
                .value
                .to_string()
        })
        .collect();
    assert_ne!(categories[0], categories[1]);
}

#[tokio::test]
async fn sibling_list_items_make_progress_without_cross_parenting_spans() {
    let capture = Capture::default();
    let context = Context {
        block: true,
        ..Default::default()
    };
    let schema = schema();
    let request =
        Request::new("{ left: nodes { child { value } } right: node { child { value } } }");
    capture
        .run(async {
            let mut execution = Box::pin(execute(&schema, &request, &Query, &context));
            for _ in 0..16 {
                assert!(futures::poll!(&mut execution).is_pending());
                if context.starts.load(Ordering::SeqCst) == 3 {
                    break;
                }
            }
            assert_eq!(
                context.starts.load(Ordering::SeqCst),
                3,
                "siblings and list items must all start before release"
            );
            context.release.store(true, Ordering::SeqCst);
            let response = serde_json::to_value(execution.await).unwrap();
            assert_eq!(
                response["data"]["left"],
                json!([{"child": {"value": 1}}, {"child": {"value": 2}}])
            );
        })
        .await;
    let spans = capture.spans();
    let children: Vec<_> = spans
        .iter()
        .filter(|span| span.name == "Node.child")
        .collect();
    assert_eq!(children.len(), 3);
    for child in children {
        let path = attr(child, "necrassrs.graphql.field.path")
            .unwrap()
            .to_string();
        child_of(
            child,
            one(
                &spans,
                if path.starts_with("left.") {
                    "Query.nodes"
                } else {
                    "Query.node"
                },
            ),
        );
        let probes: Vec<_> = spans
            .iter()
            .filter(|span| {
                span.name == "app.child" && span.parent_span_id == child.span_context.span_id()
            })
            .collect();
        assert_eq!(
            probes.len(),
            2,
            "each child resolver and its leaf must stay with their own field"
        );
    }
}

#[tokio::test]
async fn cancelling_execution_closes_spans_and_drops_all_started_futures() {
    let capture = Capture::default();
    let context = Context::default();
    capture
        .run(async {
            let schema = schema();
            let request = Request::new("{ a: hold { value } b: hold { value } }");
            let mut execution = Box::pin(execute(&schema, &request, &Query, &context));
            for _ in 0..16 {
                assert!(futures::poll!(&mut execution).is_pending());
                if context.starts.load(Ordering::SeqCst) == 2 {
                    break;
                }
            }
            assert_eq!(context.starts.load(Ordering::SeqCst), 2);
            drop(execution);
        })
        .await;
    assert_eq!(context.drops.load(Ordering::SeqCst), 2);
    let spans = capture.spans();
    let operation = one(&spans, "query");
    assert_eq!(
        attr(operation, "necrassrs.cancelled"),
        Some(&Value::Bool(true))
    );
    assert_eq!(operation.status, Status::Unset);
    assert_eq!(attr(operation, "error.type"), None);
    assert_eq!(
        spans
            .iter()
            .filter(|span| span.name == "Query.hold")
            .count(),
        2
    );
    for field in spans.iter().filter(|span| span.name == "Query.hold") {
        child_of(field, one(&spans, "necrassrs.execute"));
    }
}

#[tokio::test]
async fn mutation_null_propagation_prevents_later_fields_and_spans() {
    let capture = Capture::default();
    let context = Context::default();
    let response = capture
        .run(execute(
            &schema(),
            &Request::new("mutation { fatal second { value } }"),
            &Query,
            &context,
        ))
        .await;
    assert_eq!(serde_json::to_value(response).unwrap()["data"], json!(null));
    assert_eq!(*context.order.lock().unwrap(), ["fatal"]);
    let spans = capture.spans();
    assert!(!spans.iter().any(|span| span.name == "Mutation.second"));
    assert_eq!(
        attr(one(&spans, "mutation"), "error.type"),
        Some(&Value::from("necrassrs.execution_error"))
    );
}
