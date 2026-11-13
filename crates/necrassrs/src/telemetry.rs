use std::future::Future;

use reed_compiler::response::ResponseDataPathSegment;

use crate::Response;

#[derive(Clone)]
pub(crate) struct Span {
    #[cfg(feature = "tracing")]
    inner: tracing::Span,
}

impl Span {
    pub(crate) fn current() -> Self {
        Self {
            #[cfg(feature = "tracing")]
            inner: tracing::Span::current(),
        }
    }

    pub(crate) fn operation() -> Self {
        Self {
            #[cfg(feature = "tracing")]
            inner: tracing::info_span!(
                "GraphQL Operation",
                otel.name = tracing::field::Empty,
                otel.kind = "server",
                otel.status_code = tracing::field::Empty,
                graphql.operation.type = tracing::field::Empty,
                graphql.operation.name = tracing::field::Empty,
                error.type = tracing::field::Empty,
                necrassrs.cancelled = tracing::field::Empty,
            ),
        }
    }

    pub(crate) fn phase(name: &'static str) -> Self {
        #[cfg(not(feature = "tracing"))]
        let _ = name;
        Self {
            #[cfg(feature = "tracing")]
            inner: match name {
                "necrassrs.parse" => tracing::info_span!("necrassrs.parse"),
                "necrassrs.validate" => tracing::info_span!("necrassrs.validate"),
                "necrassrs.coerce_variables" => {
                    tracing::info_span!("necrassrs.coerce_variables")
                }
                "necrassrs.execute" => tracing::info_span!("necrassrs.execute"),
                _ => unreachable!("unknown GraphQL phase"),
            },
        }
    }

    pub(crate) fn field(parent_type: &str, field: &str, path: &str) -> Self {
        #[cfg(not(feature = "tracing"))]
        let _ = (parent_type, field, path);
        #[cfg(feature = "tracing")]
        let name = format!("{parent_type}.{field}");
        Self {
            #[cfg(feature = "tracing")]
            inner: tracing::info_span!(
                "GraphQL Field",
                otel.name = name.as_str(),
                otel.kind = "internal",
                necrassrs.graphql.field.path = path,
            ),
        }
    }

    pub(crate) fn subscription_event(parent: &Self) -> Self {
        #[cfg(not(feature = "tracing"))]
        let _ = parent;
        Self {
            #[cfg(feature = "tracing")]
            inner: tracing::info_span!(
                parent: &parent.inner,
                "necrassrs.subscription.event",
                otel.kind = "internal",
                otel.status_code = tracing::field::Empty,
                error.type = tracing::field::Empty,
            ),
        }
    }

    pub(crate) fn in_scope<T>(&self, f: impl FnOnce() -> T) -> T {
        #[cfg(feature = "tracing")]
        return self.inner.in_scope(f);
        #[cfg(not(feature = "tracing"))]
        f()
    }

    pub(crate) async fn run<F: Future>(&self, future: F) -> F::Output {
        #[cfg(feature = "tracing")]
        {
            use tracing::Instrument;
            future.instrument(self.inner.clone()).await
        }
        #[cfg(not(feature = "tracing"))]
        future.await
    }

    pub(crate) fn identify_operation(&self, operation_type: &str, operation_name: Option<&str>) {
        #[cfg(feature = "tracing")]
        {
            use opentelemetry::trace::TraceContextExt as _;
            use tracing_opentelemetry::OpenTelemetrySpanExt as _;

            self.inner.record("otel.name", operation_type);
            self.inner.record("graphql.operation.type", operation_type);
            if let Some(operation_name) = operation_name {
                self.inner.record("graphql.operation.name", operation_name);
            }
            self.inner
                .context()
                .span()
                .update_name(operation_type.to_owned());
        }
        #[cfg(not(feature = "tracing"))]
        let _ = (operation_type, operation_name);
    }

    pub(crate) fn mark_error(&self, error_type: &'static str) {
        #[cfg(feature = "tracing")]
        {
            self.inner.record("otel.status_code", "error");
            self.inner.record("error.type", error_type);
        }
        #[cfg(not(feature = "tracing"))]
        let _ = error_type;
    }

    pub(crate) fn record_error(&self, path: &[ResponseDataPathSegment], error_type: &'static str) {
        #[cfg(feature = "tracing")]
        tracing::event!(
            target: "necrassrs",
            parent: &self.inner,
            tracing::Level::ERROR,
            error.type = error_type,
            necrassrs.graphql.field.path = path_string(path),
            "GraphQL error"
        );
        #[cfg(not(feature = "tracing"))]
        let _ = (path, error_type);
    }
}

pub(crate) struct OperationSpan {
    span: Span,
    complete: bool,
}

impl OperationSpan {
    pub(crate) fn new() -> Self {
        Self {
            span: Span::operation(),
            complete: false,
        }
    }

    pub(crate) fn span(&self) -> &Span {
        &self.span
    }

    pub(crate) fn finish_response(&mut self, response: &Response) {
        if response.is_request_error() {
            self.span.mark_error("necrassrs.request_error");
        } else if response.has_errors() {
            self.span.mark_error("necrassrs.execution_error");
        }
        self.complete = true;
    }

    pub(crate) fn mark_execution_error(&self) {
        self.span.mark_error("necrassrs.execution_error");
    }

    pub(crate) fn mark_source_error(&self) {
        self.span.mark_error("necrassrs.subscription_source_error");
    }

    pub(crate) fn complete(&mut self) {
        self.complete = true;
    }
}

impl Drop for OperationSpan {
    fn drop(&mut self) {
        if !self.complete {
            #[cfg(feature = "tracing")]
            self.span.inner.record("necrassrs.cancelled", true);
        }
    }
}

pub(crate) fn path_string(path: &[ResponseDataPathSegment]) -> String {
    path.iter()
        .map(|segment| match segment {
            ResponseDataPathSegment::Field(name) => name.as_str().to_owned(),
            ResponseDataPathSegment::ListIndex(index) => index.to_string(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn record_error(path: &[ResponseDataPathSegment], error_type: &'static str) {
    #[cfg(feature = "tracing")]
    tracing::event!(
        target: "necrassrs",
        tracing::Level::ERROR,
        error.type = error_type,
        necrassrs.graphql.field.path = path_string(path),
        "GraphQL error"
    );
    #[cfg(not(feature = "tracing"))]
    let _ = (path, error_type);
}

pub(crate) fn record_request_error() {
    #[cfg(feature = "tracing")]
    tracing::event!(
        target: "necrassrs",
        tracing::Level::ERROR,
        error.type = "necrassrs.request_error",
        "GraphQL request error"
    );
}

pub(crate) async fn run<F: Future>(span: Option<Span>, future: F) -> F::Output {
    match span {
        Some(span) => span.run(future).await,
        None => future.await,
    }
}
