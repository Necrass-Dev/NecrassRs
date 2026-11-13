use std::sync::Arc;

use axum::{Router, extract::State, routing::get};
use necrassrs::{Schema, Valid};
use necrassrs_axum::{GraphQLRequest, GraphQLResponse, graphiql_html, negotiate_response};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use tower_http::trace::TraceLayer;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

mod generated;
mod resolvers;

struct AppState {
    schema: Valid<Schema>,
    dispatcher: generated::dispatch::SchemaDispatcher<resolvers::Query>,
}

async fn graphql(
    State(state): State<Arc<AppState>>,
    GraphQLRequest(request): GraphQLRequest,
) -> GraphQLResponse {
    let context = ();
    GraphQLResponse(necrassrs::execute(&state.schema, &request, &state.dispatcher, &context).await)
}

fn app() -> Router {
    let schema = Schema::parse_and_validate(generated::SDL, "embedded.graphql").unwrap();
    let state = Arc::new(AppState {
        schema,
        dispatcher: generated::dispatch::SchemaDispatcher::new(resolvers::Query),
    });
    Router::new()
        .route(
            "/graphql",
            get(graphql)
                .post(graphql)
                .route_layer(axum::middleware::from_fn(negotiate_response)),
        )
        .route("/graphiql", get(|| async { graphiql_html("/graphql") }))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

fn init_tracing() -> SdkTracerProvider {
    let provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            0.1,
        ))))
        .with_simple_exporter(opentelemetry_stdout::SpanExporter::default())
        .build();
    let subscriber = tracing_subscriber::registry().with(
        tracing_opentelemetry::layer()
            .with_error_events_to_status(false)
            .with_tracer(provider.tracer("axum-server")),
    );
    subscriber
        .try_init()
        .expect("global tracing subscriber must only be initialized once");
    provider
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let provider = init_tracing();
    let server = async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
        axum::serve(listener, app()).await
    }
    .await;
    let shutdown = provider.shutdown().map_err(std::io::Error::other);
    server?;
    shutdown
}

#[cfg(test)]
mod http_contract;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use opentelemetry::Value as OtelValue;
    use opentelemetry_sdk::trace::InMemorySpanExporter;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    #[tokio::test]
    async fn graphql_trace_is_child_of_http_trace() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(
            tracing_opentelemetry::layer()
                .with_error_events_to_status(false)
                .with_tracer(provider.tracer("axum-server-test")),
        );
        subscriber.try_init().unwrap();

        let (status, _) = post(
            &app(),
            json!({ "query": "query Example { hello(name: \"Sheri\") }" }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        let operation = spans
            .iter()
            .find(|span| {
                span.name == "query"
                    && span.attributes.iter().any(|attribute| {
                        attribute.key.as_str() == "graphql.operation.name"
                            && matches!(&attribute.value, OtelValue::String(name) if name.as_str() == "Example")
                    })
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing GraphQL operation span; exported spans: {:?}",
                    spans.iter().map(|span| span.name.as_ref()).collect::<Vec<_>>()
                )
            });
        assert!(
            spans
                .iter()
                .any(|span| span.span_context.span_id() == operation.parent_span_id),
            "GraphQL operation parent must be the exported HTTP request span"
        );
    }

    #[tokio::test]
    async fn http_method_and_transport_errors_over_tcp() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app().layer(axum::extract::DefaultBodyLimit::max(128)),
            )
            .await
            .unwrap();
        });
        let result = tokio::task::spawn_blocking(move || {
            http_contract::check_method_and_transport_errors(address);
        })
        .await;
        server.abort();
        let _ = server.await;
        result.unwrap();
    }

    async fn post(app: &Router, request: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::post("/graphql")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn literal_and_variable_requests_match() {
        let app = app();
        for request in [
            json!({ "query": "{ hello(name: \"Sheri\") }" }),
            json!({
                "query": "query($name: String!) { hello(name: $name) }",
                "variables": { "name": "Sheri" }
            }),
        ] {
            let (status, response) = post(&app, request).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(response, json!({ "data": { "hello": "Hello, Sheri" } }));
        }
    }

    #[tokio::test]
    async fn unknown_name_returns_a_field_error_and_server_recovers() {
        let app = app();
        let (status, response) =
            post(&app, json!({ "query": "{ hello(name: \"Unknown\") }" })).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            response,
            json!({
                "data": null,
                "errors": [{
                    "message": "User \"Unknown\" was not found.",
                    "locations": [{ "line": 1, "column": 3 }],
                    "path": ["hello"],
                    "extensions": { "code": "USER_NOT_FOUND" }
                }]
            })
        );

        for (request, path) in [
            (
                json!({
                    "query": "query($name: String!) { hello(name: $name) }",
                    "variables": { "name": "Unknown" }
                }),
                "hello",
            ),
            (
                json!({ "query": "{ alias: hello(name: \"Unknown\") }" }),
                "alias",
            ),
        ] {
            let (status, response) = post(&app, request).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(response["data"], Value::Null);
            assert_eq!(
                response["errors"][0]["message"],
                "User \"Unknown\" was not found."
            );
            assert_eq!(response["errors"][0]["path"], json!([path]));
            assert_eq!(
                response["errors"][0]["extensions"]["code"],
                "USER_NOT_FOUND"
            );
            assert!(
                response["errors"][0]["locations"]
                    .as_array()
                    .is_some_and(|locations| !locations.is_empty())
            );
        }

        let (status, response) = post(&app, json!({ "query": "{ hello(name: \"Sheri\") }" })).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response, json!({ "data": { "hello": "Hello, Sheri" } }));
    }

    #[tokio::test]
    async fn invalid_inputs_are_request_errors() {
        let app = app();
        for request in [
            json!({ "query": "{ hello }" }),
            json!({ "query": "{ hello(name: null) }" }),
            json!({ "query": "{ hello(name: 3) }" }),
            json!({
                "query": "query($name: String!) { hello(name: $name) }",
                "variables": {}
            }),
            json!({
                "query": "query($name: String!) { hello(name: $name) }",
                "variables": { "name": null }
            }),
            json!({
                "query": "query($name: String!) { hello(name: $name) }",
                "variables": { "name": 3 }
            }),
        ] {
            let (status, response) = post(&app, request).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(response.get("data").is_none());
            assert!(
                response["errors"]
                    .as_array()
                    .is_some_and(|errors| !errors.is_empty())
            );
        }
    }

    #[tokio::test]
    async fn names_are_matched_exactly() {
        let app = app();
        for name in ["sheri", " Sheri "] {
            let (status, response) = post(
                &app,
                json!({
                    "query": "query($name: String!) { hello(name: $name) }",
                    "variables": { "name": name }
                }),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(response["data"], Value::Null);
            assert_eq!(
                response["errors"][0]["extensions"]["code"],
                "USER_NOT_FOUND"
            );
        }
    }

    #[tokio::test]
    async fn graphiql_page_uses_a_separate_ui_route() {
        let response = app()
            .oneshot(Request::get("/graphiql").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
        let html = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let html = std::str::from_utf8(&html).unwrap();
        assert!(html.contains("data-endpoint=\"/graphql\""));
    }

    #[tokio::test]
    async fn graphql_get_returns_query_data_instead_of_graphiql() {
        let response = app()
            .oneshot(
                Request::get("/graphql?query=%7B__typename%7D")
                    .header(header::ACCEPT, "application/graphql-response+json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .split(';')
                .next(),
            Some("application/graphql-response+json"),
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body, json!({ "data": { "__typename": "Query" } }));
    }
}
