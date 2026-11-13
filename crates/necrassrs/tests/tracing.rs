use necrassrs::{Request, execute};
use opentelemetry::{
    KeyValue,
    trace::{SpanKind, Status},
};
use serde_json::json;

#[path = "tracing/fields.rs"]
mod fields;
#[path = "tracing/fixture.rs"]
pub mod fixture;
#[path = "tracing/operations.rs"]
mod operations;
#[path = "tracing/subscriptions.rs"]
mod subscriptions;
#[path = "tracing/support.rs"]
pub mod support;

use fixture::{Context, Query, schema};
use support::{Capture, one};

#[tokio::test]
async fn successful_named_query_exports_operation_span() {
    let capture = Capture::default();
    let response = capture
        .run(execute(
            &schema(),
            &Request::new("query Ping { ping }"),
            &Query,
            &Context::default(),
        ))
        .await;
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({"data": {"ping": 42}})
    );
    let spans = capture.spans();
    let operation = one(&spans, "query");
    assert_eq!(operation.span_kind, SpanKind::Server);
    assert_eq!(operation.status, Status::Unset);
    assert!(
        operation
            .attributes
            .contains(&KeyValue::new("graphql.operation.type", "query"))
    );
    assert!(
        operation
            .attributes
            .contains(&KeyValue::new("graphql.operation.name", "Ping"))
    );
}
