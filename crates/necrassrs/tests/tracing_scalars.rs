use futures::StreamExt;
use necrassrs::{
    ExecutionOptions, Request, ResolvedValue, SourceEvent, execute_with_options,
    subscribe_with_options,
};
use opentelemetry::{Value, trace::SpanKind};
use serde_json::json;

#[path = "tracing/fixture.rs"]
pub mod fixture;
#[path = "tracing/support.rs"]
pub mod support;

use fixture::{Context, Events, Query, node, schema};
use support::{Capture, attr, child_of, one, send};

#[test]
fn scalar_tracing_is_opt_in() {
    assert!(!ExecutionOptions::default().trace_scalars);
}

#[tokio::test]
async fn scalar_opt_in_includes_wrapped_scalars_enums_and_abstract_object_fields() {
    let capture = Capture::default();
    let request = Request::new(
        "{ ping custom state scalars enums node { value } named { value } search { ... on Node { value } } __typename __schema { queryType { name } } }",
    );
    let response = capture
        .run(execute_with_options(
            &schema(),
            &request,
            &Query,
            &Context::default(),
            ExecutionOptions {
                trace_scalars: true,
                ..Default::default()
            },
        ))
        .await;
    let response = serde_json::to_value(response).unwrap();
    assert!(response.get("errors").is_none(), "{response}");
    assert_eq!(
        response["data"]["search"],
        json!([{"value": 1}, {"value": 2}])
    );
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
        ("Query.ping", "ping"),
        ("Query.custom", "custom"),
        ("Query.state", "state"),
        ("Query.scalars", "scalars"),
        ("Query.enums", "enums"),
        ("Query.node", "node"),
        ("Node.value", "node.value"),
        ("Query.named", "named"),
        ("Node.value", "named.value"),
        ("Query.search", "search"),
        ("Node.value", "search.0.value"),
        ("Node.value", "search.1.value"),
    ]
    .into_iter()
    .map(|(name, path)| (name.to_owned(), path.to_owned()))
    .collect();
    expected.sort();
    assert_eq!(
        fields, expected,
        "introspection and list-item-only spans remain excluded"
    );
    for field in spans
        .iter()
        .filter(|span| attr(span, "necrassrs.graphql.field.path").is_some())
    {
        assert_eq!(field.span_kind, SpanKind::Internal);
    }
}

#[tokio::test]
async fn opted_in_scalar_error_event_is_attached_once_to_its_field() {
    let capture = Capture::default();
    let response = capture
        .run(execute_with_options(
            &schema(),
            &Request::new("{ alias: fail }"),
            &Query,
            &Context::default(),
            ExecutionOptions {
                trace_scalars: true,
                ..Default::default()
            },
        ))
        .await;
    assert_eq!(
        serde_json::to_value(response).unwrap()["data"],
        json!({"alias": null})
    );
    let spans = capture.spans();
    let field = one(&spans, "Query.fail");
    assert_eq!(
        attr(field, "necrassrs.graphql.field.path"),
        Some(&Value::from("alias"))
    );
    assert_eq!(field.events.len(), 1);
    assert_eq!(spans.iter().map(|span| span.events.len()).sum::<usize>(), 1);
    child_of(field, one(&spans, "necrassrs.execute"));
}

#[tokio::test]
async fn subscription_scalar_option_applies_to_root_and_nested_event_fields() {
    for (document, value, root, leaf) in [
        (
            "subscription { scalar }",
            ResolvedValue::Json(7.into()),
            "Subscription.scalar",
            None,
        ),
        (
            "subscription { events { value } }",
            node(7),
            "Subscription.events",
            Some("Node.value"),
        ),
    ] {
        let capture = Capture::default();
        capture
            .run(async {
                let schema = schema();
                let context = Context::default();
                let dispatcher = Events::new(vec![SourceEvent::Value(value)]);
                let mut responses = send(
                    subscribe_with_options(
                        &schema,
                        &Request::new(document),
                        &dispatcher,
                        &context,
                        ExecutionOptions {
                            trace_scalars: true,
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap_or_else(|_| panic!("startup failed")),
                );
                let response =
                    serde_json::to_value(responses.next().await.unwrap().unwrap()).unwrap();
                assert!(response.get("errors").is_none());
                assert!(responses.next().await.is_none());
            })
            .await;
        let spans = capture.spans();
        let field = one(&spans, root);
        child_of(field, one(&spans, "necrassrs.subscription.event"));
        if let Some(leaf) = leaf {
            child_of(one(&spans, leaf), field);
            assert_eq!(
                attr(one(&spans, leaf), "necrassrs.graphql.field.path"),
                Some(&Value::from("events.value"))
            );
        }
    }
}
