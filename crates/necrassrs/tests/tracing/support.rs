use std::future::Future;

use opentelemetry::{Value, trace::TracerProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::SubscriberExt;

pub struct Capture {
    provider: SdkTracerProvider,
    exporter: InMemorySpanExporter,
    pub dispatch: tracing::Dispatch,
}

impl Default for Capture {
    fn default() -> Self {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(
            tracing_opentelemetry::layer()
                .with_error_events_to_status(false)
                .with_tracer(provider.tracer("necrassrs-test")),
        );
        let dispatch = tracing::Dispatch::new(subscriber);
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info_span!("test.exporter_probe").in_scope(|| {});
        });
        Self {
            provider,
            exporter,
            dispatch,
        }
    }
}

impl Capture {
    pub async fn run<F: Future + Send>(&self, future: F) -> F::Output {
        future.with_subscriber(self.dispatch.clone()).await
    }

    pub fn spans(&self) -> Vec<SpanData> {
        self.provider.force_flush().unwrap();
        let mut spans = self.exporter.get_finished_spans().unwrap();
        assert!(
            spans.iter().any(|span| span.name == "test.exporter_probe"),
            "the local subscriber must export spans before testing instrumentation"
        );
        spans.retain(|span| span.name != "test.exporter_probe");
        spans
    }
}

pub fn attr<'a>(span: &'a SpanData, key: &str) -> Option<&'a Value> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| &kv.value)
}

pub fn one<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
    let matching: Vec<_> = spans.iter().filter(|span| span.name == name).collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one {name} span; exported names: {:?}",
        spans
            .iter()
            .map(|span| span.name.as_ref())
            .collect::<Vec<_>>()
    );
    matching[0]
}

pub fn child_of(child: &SpanData, parent: &SpanData) {
    assert_eq!(
        child.parent_span_id,
        parent.span_context.span_id(),
        "{} parent",
        child.name
    );
    assert_eq!(
        child.span_context.trace_id(),
        parent.span_context.trace_id()
    );
    assert!(
        parent.start_time <= child.start_time,
        "{} starts inside {}",
        child.name,
        parent.name
    );
    assert!(
        child.end_time <= parent.end_time,
        "{} ends inside {}",
        child.name,
        parent.name
    );
}

pub fn send<T: Send>(value: T) -> T {
    value
}
