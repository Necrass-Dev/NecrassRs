use actix_web::{App, HttpServer, web};
use necrassrs::{Schema, Valid};
use necrassrs_actix::{GraphQLRequest, GraphQLResponse, graphiql_html};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use tracing_actix_web::TracingLogger;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

mod generated;
mod resolvers;

struct AppState {
    schema: Valid<Schema>,
    dispatcher: generated::dispatch::SchemaDispatcher<resolvers::Query>,
}

async fn graphql(
    state: web::Data<AppState>,
    GraphQLRequest(request): GraphQLRequest,
) -> GraphQLResponse {
    let context = ();
    GraphQLResponse(necrassrs::execute(&state.schema, &request, &state.dispatcher, &context).await)
}

fn configure(cfg: &mut web::ServiceConfig) {
    let schema = Schema::parse_and_validate(generated::SDL, "embedded.graphql").unwrap();
    cfg.app_data(web::Data::new(AppState {
        schema,
        dispatcher: generated::dispatch::SchemaDispatcher::new(resolvers::Query),
    }))
    .service(
        web::resource("/graphql")
            .route(web::post().to(graphql))
            .route(web::get().to(graphql)),
    )
    .route(
        "/graphiql",
        web::get().to(|| async { graphiql_html("/graphql") }),
    );
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
            .with_tracer(provider.tracer("actix-server")),
    );
    subscriber
        .try_init()
        .expect("global tracing subscriber must only be initialized once");
    provider
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let provider = init_tracing();
    let server = async {
        HttpServer::new(|| {
            App::new()
                .wrap(TracingLogger::default())
                .configure(configure)
        })
        .bind(("127.0.0.1", 3001))?
        .run()
        .await
    }
    .await;
    let shutdown = provider.shutdown().map_err(std::io::Error::other);
    server?;
    shutdown
}

#[cfg(test)]
#[path = "../../http_contract.rs"]
mod http_contract;

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{
        http::{StatusCode, header},
        test,
    };
    use opentelemetry::Value as OtelValue;
    use opentelemetry_sdk::trace::InMemorySpanExporter;
    use serde_json::{Value, json};

    #[actix_web::test]
    async fn graphql_trace_is_child_of_http_trace() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(
            tracing_opentelemetry::layer()
                .with_error_events_to_status(false)
                .with_tracer(provider.tracer("actix-server-test")),
        );
        subscriber.try_init().unwrap();

        let app = test::init_service(
            App::new()
                .wrap(TracingLogger::default())
                .configure(configure),
        )
        .await;
        let request = test::TestRequest::post()
            .uri("/graphql")
            .set_json(json!({
                "query": "query Example { hello(name: \"Sheri\") }"
            }))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::OK);
        test::read_body(response).await;

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

    #[actix_web::test]
    async fn http_method_and_transport_errors_over_tcp() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = HttpServer::new(|| {
            App::new()
                .configure(configure)
                .app_data(web::JsonConfig::default().limit(128))
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        let server = actix_web::rt::spawn(server);
        let result = actix_web::rt::task::spawn_blocking(move || {
            // Actix maps URI overflow to 400 in its HTTP/1 parser before application dispatch.
            http_contract::check_method_and_transport_errors(address, 400);
        })
        .await;
        handle.stop(false).await;
        server.await.unwrap().unwrap();
        result.unwrap();
    }

    #[actix_web::test]
    async fn generated_resolvers_accept_literals_variables_and_operation_names() {
        let app = test::init_service(App::new().configure(configure)).await;
        for body in [
            json!({ "query": "{ hello(name: \"Sheri\") }" }),
            json!({
                "query": "query First { hello(name: \"Unknown\") } query Second($name: String!) { hello(name: $name) }",
                "variables": { "name": "Sheri" },
                "operationName": "Second"
            }),
        ] {
            let request = test::TestRequest::post()
                .uri("/graphql")
                .insert_header((header::ACCEPT, "application/graphql-response+json"))
                .set_json(body)
                .to_request();
            let response = test::call_service(&app, request).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "application/graphql-response+json"
            );
            let body: Value = test::read_body_json(response).await;
            assert_eq!(body, json!({ "data": { "hello": "Hello, Sheri" } }));
        }
    }

    #[actix_web::test]
    async fn generated_resolver_errors_propagate_and_later_requests_succeed() {
        let app = test::init_service(App::new().configure(configure)).await;
        for (query, status, expected) in [
            (
                "{ alias: hello(name: \"Unknown\") }",
                StatusCode::OK,
                json!({ "data": null, "errors": [{
                    "message": "User \"Unknown\" was not found.",
                    "locations": [{ "line": 1, "column": 10 }],
                    "path": ["alias"], "extensions": { "code": "USER_NOT_FOUND" }
                }] }),
            ),
            (
                "{ hello(name: \"Sheri\") }",
                StatusCode::OK,
                json!({ "data": { "hello": "Hello, Sheri" } }),
            ),
        ] {
            let request = test::TestRequest::post()
                .uri("/graphql")
                .set_json(json!({ "query": query }))
                .to_request();
            let response = test::call_service(&app, request).await;
            assert_eq!(response.status(), status);
            let body: Value = test::read_body_json(response).await;
            assert_eq!(body, expected);
        }
    }

    #[actix_web::test]
    async fn built_in_graphiql_uses_a_separate_ui_route() {
        let app = test::init_service(App::new().configure(configure)).await;
        let request = test::TestRequest::get()
            .uri("/graphiql")
            .insert_header((header::ACCEPT, "text/html"))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        let html = test::read_body(response).await;
        assert!(
            std::str::from_utf8(&html)
                .unwrap()
                .contains("data-endpoint=\"/graphql\"")
        );

        let request = test::TestRequest::post()
            .uri("/graphql")
            .set_json(json!({ "query": "{ __schema { queryType { name } } }" }))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = test::read_body_json(response).await;
        assert_eq!(
            body,
            json!({ "data": { "__schema": { "queryType": { "name": "Query" } } } })
        );
    }

    #[actix_web::test]
    async fn graphql_get_returns_query_data_instead_of_graphiql() {
        let app = test::init_service(App::new().configure(configure)).await;
        let request = test::TestRequest::get()
            .uri("/graphql?query=%7B__typename%7D")
            .insert_header((header::ACCEPT, "application/graphql-response+json"))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap()
                .split(';')
                .next(),
            Some("application/graphql-response+json"),
        );
        let body: Value = test::read_body_json(response).await;
        assert_eq!(body, json!({ "data": { "__typename": "Query" } }));
    }
}
