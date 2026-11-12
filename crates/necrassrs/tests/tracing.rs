use necrassrs::{
    Dispatcher, FieldCoordinate, JsonMap, Request, ResolvedValue, ResolverError, Schema, execute,
};
use opentelemetry::{
    KeyValue,
    trace::{SpanKind, Status, TracerProvider},
};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use serde_json::json;
use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::SubscriberExt;

struct Query;

impl Dispatcher<()> for Query {
    async fn resolve<'a>(
        &'a self,
        _: &'a (),
        coordinate: FieldCoordinate<'a>,
        _: &'a JsonMap,
    ) -> Result<ResolvedValue, ResolverError> {
        assert_eq!(coordinate.parent_type, "Query");
        assert_eq!(coordinate.field, "ping");
        Ok(ResolvedValue::Json(42.into()))
    }
}

#[tokio::test]
async fn successful_named_query_exports_operation_span() {
    let schema = Schema::parse_and_validate("type Query { ping: Int! }", "schema.graphql").unwrap();
    let request = Request::new("query Ping { ping }");
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("necrassrs-test")));

    let response = async {
        tracing::info_span!("test.exporter_probe").in_scope(|| {});
        execute(&schema, &request, &Query, &()).await
    }
    .with_subscriber(subscriber)
    .await;
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({ "data": { "ping": 42 } })
    );

    provider.force_flush().unwrap();
    let spans = exporter.get_finished_spans().unwrap();
    assert!(
        spans.iter().any(|span| span.name == "test.exporter_probe"),
        "the local subscriber must export spans before checking runtime instrumentation"
    );
    let operations: Vec<_> = spans.iter().filter(|span| span.name == "query").collect();
    assert_eq!(
        operations.len(),
        1,
        "a successful named query must export exactly one GraphQL operation span"
    );
    let operation = operations[0];
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
