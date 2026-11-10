use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Request as HttpRequest, StatusCode, header},
    routing::{get, post},
};
use necrassrs::{
    Dispatcher, FieldCoordinate, JsonMap, JsonValue, ResolvedValue, ResolverError,
    ScalarLiteralMap, Schema, SourceStream, Valid, execute,
};
use necrassrs_axum::{GraphQLRequest, GraphQLResponse, graphiql_html, negotiate_response};
use serde_json::{Value, json};
use tower::ServiceExt;

struct AppState {
    schema: Valid<Schema>,
    dispatcher: GreetingDispatcher,
}

#[derive(Default)]
struct GreetingDispatcher {
    calls: AtomicUsize,
    sources: AtomicUsize,
}

struct Context {
    prefix: String,
}

impl Dispatcher<Context> for GreetingDispatcher {
    async fn resolve<'a>(
        &'a self,
        context: &'a Context,
        coordinate: FieldCoordinate<'a>,
        arguments: &'a JsonMap,
    ) -> Result<ResolvedValue<Context>, ResolverError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(matches!(coordinate.field, "hello" | "nullableHello"));
        let name = arguments.get("name").and_then(JsonValue::as_str).unwrap();

        if name == "Unknown" {
            return Err(ResolverError::new("User \"Unknown\" was not found.")
                .with_extension("code", "USER_NOT_FOUND"));
        }

        Ok(ResolvedValue::Json(
            format!("{}, {name}", context.prefix).into(),
        ))
    }

    async fn resolve_event_stream<'stream, 'call>(
        &'stream self,
        _: &'stream Context,
        _: FieldCoordinate<'call>,
        _: &'call JsonMap,
        _: &'call ScalarLiteralMap,
    ) -> Result<SourceStream<'stream, ResolvedValue<Context>>, ResolverError>
    where
        'stream: 'call,
    {
        self.sources.fetch_add(1, Ordering::SeqCst);
        Err(ResolverError::new("Source creation must not be reached"))
    }
}

async fn graphql(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    GraphQLRequest(request): GraphQLRequest,
) -> GraphQLResponse {
    let context = Context {
        prefix: headers
            .get("x-prefix")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("Hello")
            .to_owned(),
    };
    GraphQLResponse(execute(&state.schema, &request, &state.dispatcher, &context).await)
}

fn app() -> Router {
    app_at("/graphql")
}

fn app_at(endpoint: &str) -> Router {
    let schema = Schema::parse_and_validate(
        "type Query { hello(name: String!): String! nullableHello(name: String!): String }",
        "schema.graphql",
    )
    .unwrap();
    Router::new()
        .route(
            endpoint,
            post(graphql).route_layer(axum::middleware::from_fn(negotiate_response)),
        )
        .with_state(Arc::new(AppState {
            schema,
            dispatcher: GreetingDispatcher::default(),
        }))
}

async fn send(
    app: &Router,
    method: &str,
    content_type: Option<&str>,
    prefix: Option<&str>,
    body: impl Into<Body>,
) -> (StatusCode, Option<String>, Value) {
    send_at(app, "/graphql", method, content_type, prefix, body).await
}

async fn send_at(
    app: &Router,
    endpoint: &str,
    method: &str,
    content_type: Option<&str>,
    prefix: Option<&str>,
    body: impl Into<Body>,
) -> (StatusCode, Option<String>, Value) {
    let mut builder = HttpRequest::builder()
        .method(method)
        .uri(endpoint)
        .header(header::ACCEPT, "application/json");
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    if let Some(prefix) = prefix {
        builder = builder.header("x-prefix", prefix);
    }
    let response = app
        .clone()
        .oneshot(builder.body(body.into()).unwrap())
        .await
        .unwrap();
    response_parts(response).await
}

async fn response_parts(response: axum::response::Response) -> (StatusCode, Option<String>, Value) {
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|value| value.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, content_type, body)
}

#[tokio::test]
async fn extracts_operation_variables_and_per_request_context() {
    let app = app();
    let body = json!({
        "query": "query First { hello(name: \"Skip\") } query Second($name: String!) { hello(name: $name) }",
        "operationName": "Second",
        "variables": { "name": "Sheri" }
    })
    .to_string();

    let (status, media_type, response) = send(
        &app,
        "POST",
        Some("application/json"),
        Some("Hi"),
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(media_type.as_deref(), Some("application/json"));
    assert_eq!(response, json!({ "data": { "hello": "Hi, Sheri" } }));

    let (_, _, response) = send(&app, "POST", Some("application/json"), None, body).await;
    assert_eq!(response, json!({ "data": { "hello": "Hello, Sheri" } }));
}

#[tokio::test]
async fn distinguishes_request_and_execution_errors_and_recovers() {
    let app = app();

    let (status, _, response) = send(
        &app,
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{ hello(name: 3) }" }).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.get("data").is_none());
    assert!(response.get("errors").is_some());

    let (status, _, response) = send(
        &app,
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{ alias: hello(name: \"Unknown\") }" }).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["data"], Value::Null);
    assert_eq!(
        response["errors"][0]["message"],
        "User \"Unknown\" was not found."
    );
    assert_eq!(response["errors"][0]["path"], json!(["alias"]));
    assert_eq!(
        response["errors"][0]["extensions"]["code"],
        "USER_NOT_FOUND"
    );

    let (status, _, response) = send(
        &app,
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{ hello(name: \"Sheri\") }" }).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response, json!({ "data": { "hello": "Hello, Sheri" } }));
}

#[tokio::test]
async fn route_only_accepts_post() {
    let app = app();
    for accept in [None, Some("text/html")] {
        let mut request = HttpRequest::get("/graphql");
        if let Some(accept) = accept {
            request = request.header(header::ACCEPT, accept);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers().get(header::ALLOW).unwrap(), "POST");
    }
}

#[tokio::test]
async fn schema_discovery_and_queries_share_the_existing_graphql_endpoint() {
    let app = app();
    let (status, _, response) = send(
        &app,
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{ __schema { queryType { name } } }" }).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response,
        json!({ "data": { "__schema": { "queryType": { "name": "Query" } } } })
    );

    let (status, _, response) = send(
        &app,
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{ hello(name: \"Sheri\") }" }).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response, json!({ "data": { "hello": "Hello, Sheri" } }));
}

#[tokio::test]
async fn graphiql_is_opt_in_and_uses_the_configured_endpoint() {
    let request = || {
        HttpRequest::get("/api/graphql")
            .body(Body::empty())
            .unwrap()
    };
    let absent = app_at("/api/graphql").oneshot(request()).await.unwrap();
    assert_eq!(absent.status(), StatusCode::METHOD_NOT_ALLOWED);

    let app = app_at("/api/graphql").route(
        "/api/graphql",
        get(|| async { graphiql_html("/api/graphql") }),
    );
    let page = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(
        page.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );
    let html = to_bytes(page.into_body(), usize::MAX).await.unwrap();
    let html = std::str::from_utf8(&html).unwrap();
    assert!(html.contains("/api/graphql"));
    assert!(html.contains("rel=\"stylesheet\""));
    assert!(html.contains("type=\"module\""));
    assert!(html.contains("createGraphiQLFetcher({ url: element.dataset.endpoint })"));

    for (query, expected) in [
        (
            "{ __schema { queryType { name } } }",
            json!({ "data": { "__schema": { "queryType": { "name": "Query" } } } }),
        ),
        (
            "{ hello(name: \"Sheri\") }",
            json!({ "data": { "hello": "Hello, Sheri" } }),
        ),
    ] {
        let (status, _, response) = send_at(
            &app,
            "/api/graphql",
            "POST",
            Some("application/json"),
            None,
            json!({ "query": query }).to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response, expected);
    }
}

// HTTP acceptance cases informed by graphql-over-http revision 3903e680.
// These do not establish complete conformance or a policy for missing Accept.
#[tokio::test]
async fn graphql_syntax_errors_return_400_without_data() {
    let app = app();
    let (status, _, response_body) = send(
        &app,
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{" }).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(response_body.get("data").is_none());
    assert!(
        response_body["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
    );
}

#[tokio::test]
async fn validation_operation_selection_and_variable_errors_return_422() {
    let app = app();
    for body in [
        json!({ "query": "{ missing }" }),
        json!({ "query": "query A { hello(name: \"Sheri\") } query B { hello(name: \"Sheri\") }" }),
        json!({ "query": "query A { hello(name: \"Sheri\") }", "operationName": "Missing" }),
        json!({ "query": "query($name: String!) { hello(name: $name) }", "variables": {} }),
        json!({ "query": "query($name: String!) { hello(name: $name) }", "variables": { "name": null } }),
        json!({ "query": "query($name: String!) { hello(name: $name) }", "variables": { "name": 3 } }),
    ] {
        let (status, _, response_body) = send(
            &app,
            "POST",
            Some("application/json"),
            None,
            body.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(response_body.get("data").is_none(), "{body}");
        assert!(
            response_body["errors"]
                .as_array()
                .is_some_and(|errors| !errors.is_empty()),
            "{body}"
        );
    }
}

#[tokio::test]
async fn partial_execution_errors_remain_http_200() {
    let app = app();
    // Maintainer decision in #18: execution errors use 200, never 294.
    let body = json!({
        "query": "{ ok: hello(name: \"Sheri\") failed: nullableHello(name: \"Unknown\") }"
    })
    .to_string();
    let (status, _, response_body) = send(&app, "POST", Some("application/json"), None, body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response_body["data"],
        json!({ "ok": "Hello, Sheri", "failed": null })
    );
    assert_eq!(response_body["errors"].as_array().unwrap().len(), 1);
    assert_eq!(response_body["errors"][0]["path"], json!(["failed"]));
    assert_eq!(
        response_body["errors"][0]["extensions"]["code"],
        "USER_NOT_FOUND"
    );
}

#[tokio::test]
async fn negotiates_graphql_response_media_type() {
    let app = app();
    for (accept, expected) in [
        (
            "application/graphql-response+json",
            "application/graphql-response+json",
        ),
        (
            "application/json;q=0.5, application/graphql-response+json;q=1",
            "application/graphql-response+json",
        ),
        (
            "application/graphql-response+json;q=0.5, application/json;q=1",
            "application/json",
        ),
        (
            "application/graphql-response+json;q=0, application/json",
            "application/json",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                HttpRequest::post("/graphql")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ACCEPT, accept)
                    .body(Body::from(
                        json!({ "query": "{ hello(name: \"Sheri\") }" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, media_type, response_body) = response_parts(response).await;
        assert_eq!(status, StatusCode::OK, "{accept}");
        // An optional charset parameter must not make the media-type check fail.
        assert_eq!(
            media_type
                .as_deref()
                .and_then(|value| value.split(';').next())
                .map(str::trim),
            Some(expected),
            "{accept}"
        );
        assert_eq!(
            response_body,
            json!({ "data": { "hello": "Hello, Sheri" } })
        );
    }
}

#[tokio::test]
async fn malformed_json_returns_400() {
    let app = app();
    let (status, _, _) = send(&app, "POST", Some("application/json"), None, "{").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn invalid_request_fields_return_422() {
    let app = app();
    for body in [
        json!({}),
        json!({ "query": 3 }),
        json!({ "query": "{ __typename }", "variables": [] }),
        json!({ "query": "{ __typename }", "operationName": 3 }),
    ] {
        let (status, _, _) = send(
            &app,
            "POST",
            Some("application/json"),
            None,
            body.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
}

#[tokio::test]
async fn unsupported_request_media_type_returns_415() {
    let app = app();
    let (status, _, _) = send(
        &app,
        "POST",
        Some("text/plain"),
        None,
        json!({ "query": "{ __typename }" }).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn unacceptable_response_type_is_rejected_before_body_extraction() {
    let response = app()
        .oneshot(
            HttpRequest::post("/graphql")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "text/html")
                .body(Body::from("{"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
    assert_eq!(response.headers().get(header::VARY).unwrap(), "Accept");
    assert!(response.headers().get(header::CONTENT_TYPE).is_none());
}

// Issue #30: expected HTTP contracts; implementations intentionally remain unchanged.

fn contract_app() -> (Router, Arc<AppState>) {
    let state = Arc::new(AppState {
        schema: Schema::parse_and_validate(
            "type Query { hello(name: String!): String! }
             type Mutation { hello(name: String!): String! }
             type Subscription { hello(name: String!): String! }",
            "schema.graphql",
        )
        .unwrap(),
        dispatcher: GreetingDispatcher::default(),
    });
    let app = Router::new()
        .route(
            "/graphql",
            get(graphql)
                .post(graphql)
                .route_layer(axum::middleware::from_fn(negotiate_response)),
        )
        .with_state(state.clone());
    (app, state)
}

#[tokio::test]
async fn array_envelopes_are_rejected_before_dispatch() {
    let (app, state) = contract_app();
    let body = json!(["{ hello(name: \"Sheri\") }", null, null]).to_string();
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/graphql")
        .header(header::ACCEPT, "application/json")
        .header(header::CONTENT_TYPE, "application/json");
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let (status, _, _) = response_parts(response).await;
    assert_eq!(
        (status, state.dispatcher.calls.load(Ordering::SeqCst)),
        (StatusCode::UNPROCESSABLE_ENTITY, 0)
    );
}

#[tokio::test]
async fn non_object_extensions_are_rejected_before_dispatch() {
    let (app, state) = contract_app();
    for extensions in [Value::Null, json!({}), json!({ "trace": true })] {
        let (status, _, body) = send(
            &app,
            "POST",
            Some("application/json"),
            None,
            json!({
                "query": "{ hello(name: \"Sheri\") }",
                "variables": null,
                "operationName": null,
                "extensions": extensions,
                "unknown": true,
            })
            .to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "data": { "hello": "Hello, Sheri" } }));
    }
    state.dispatcher.calls.store(0, Ordering::SeqCst);
    for extensions in [json!([]), json!(123), json!("invalid"), json!(true)] {
        let body =
            json!({ "query": "{ hello(name: \"Sheri\") }", "extensions": extensions }).to_string();
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/graphql")
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/json");
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let (status, _, _) = response_parts(response).await;
        assert_eq!(
            (status, state.dispatcher.calls.load(Ordering::SeqCst)),
            (StatusCode::UNPROCESSABLE_ENTITY, 0),
            "{extensions}"
        );
    }
}

#[tokio::test]
async fn get_decodes_operation_variables_and_extensions() {
    let (app, state) = contract_app();
    let uri = "/graphql?query=query+First%7Bhello(name%3A%22Skip%22)%7D+query+Second(%24name%3AString!)%7Bhello(name%3A%24name)%7D&operationName=Second&variables=%7B%22name%22%3A%22Tachibana+Sheri%2B%22%7D&extensions=%7B%22trace%22%3Atrue%7D";
    let request = HttpRequest::builder()
        .method("GET")
        .uri(uri)
        .header(header::ACCEPT, "application/json");
    let response = app
        .clone()
        .oneshot(request.body(Body::from("")).unwrap())
        .await
        .unwrap();
    let (status, media_type, body) = response_parts(response).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        media_type.as_deref().unwrap().split(';').next(),
        Some("application/json")
    );
    assert_eq!(
        body,
        json!({ "data": { "hello": "Hello, Tachibana Sheri+" } })
    );
    assert_eq!(state.dispatcher.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn get_normalizes_empty_optional_parameters() {
    let (app, state) = contract_app();
    let uri =
        "/graphql?query=%7Bhello(name%3A%22Sheri%22)%7D&operationName=&variables=&extensions=";
    let request = HttpRequest::builder()
        .method("GET")
        .uri(uri)
        .header(header::ACCEPT, "application/json");
    let response = app
        .clone()
        .oneshot(request.body(Body::from("")).unwrap())
        .await
        .unwrap();
    let (status, _, body) = response_parts(response).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "data": { "hello": "Hello, Sheri" } }));
    assert_eq!(state.dispatcher.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn get_mutations_are_rejected_before_dispatch_even_with_a_json_body() {
    let (app, state) = contract_app();
    let uri = "/graphql?query=mutation+Update%7Bhello(name%3A%22Sheri%22)%7D&operationName=Update";
    // A JSON body must not bypass the safe-method restriction.
    let body =
        json!({ "query": "mutation Update { hello(name: \"Sheri\") }", "operationName": "Update" })
            .to_string();
    let request = HttpRequest::builder()
        .method("GET")
        .uri(uri)
        .header(header::ACCEPT, "application/json")
        .header(header::CONTENT_TYPE, "application/json");
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (
            response.status(),
            state.dispatcher.calls.load(Ordering::SeqCst)
        ),
        (StatusCode::METHOD_NOT_ALLOWED, 0)
    );
    assert!(
        response
            .headers()
            .get(header::ALLOW)
            .unwrap()
            .to_str()
            .unwrap()
            .split(',')
            .any(|method| method.trim() == "POST")
    );
}

#[tokio::test]
async fn request_errors_preserve_negotiated_json_for_clients_supporting_both_types() {
    let (app, state) = contract_app();
    for (query, expected_status) in [
        ("{", StatusCode::BAD_REQUEST),
        ("{ missing }", StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let body = json!({ "query": query }).to_string();
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/graphql")
            .header(
                header::ACCEPT,
                "application/json;q=1, application/graphql-response+json;q=0.1",
            )
            .header(header::CONTENT_TYPE, "application/json");
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let (status, media_type, body) = response_parts(response).await;
        assert_eq!(status, expected_status);
        assert_eq!(
            media_type.as_deref().unwrap().split(';').next(),
            Some("application/json")
        );
        assert!(body.get("data").is_none());
        assert!(
            body["errors"]
                .as_array()
                .is_some_and(|errors| !errors.is_empty())
        );
        assert_eq!(state.dispatcher.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn json_subscriptions_return_406_without_creating_a_source() {
    let (app, state) = contract_app();
    for accept in ["application/json", "application/graphql-response+json"] {
        let body = json!({ "query": "subscription { hello(name: \"Sheri\") }" }).to_string();
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/graphql")
            .header(header::ACCEPT, accept)
            .header(header::CONTENT_TYPE, "application/json");
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let (status, media_type, body) = response_parts(response).await;
        assert_eq!(
            (
                status,
                state.dispatcher.sources.load(Ordering::SeqCst),
                state.dispatcher.calls.load(Ordering::SeqCst)
            ),
            (StatusCode::NOT_ACCEPTABLE, 0, 0)
        );
        assert_eq!(
            media_type.as_deref().unwrap().split(';').next(),
            Some("application/graphql-response+json")
        );
        assert!(body.get("data").is_none());
        assert!(
            body["errors"]
                .as_array()
                .is_some_and(|errors| !errors.is_empty())
        );
    }
}

#[tokio::test]
async fn head_mutations_are_rejected_before_dispatch() {
    let (app, state) = contract_app();
    let response = app
        .oneshot(
            HttpRequest::builder()
                .method("HEAD")
                .uri("/graphql")
                .header(header::ACCEPT, "application/json")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "query": "mutation { hello(name: \"Sheri\") }" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        (
            response.status(),
            state.dispatcher.calls.load(Ordering::SeqCst)
        ),
        (StatusCode::METHOD_NOT_ALLOWED, 0)
    );
    assert!(
        response.headers()[header::ALLOW]
            .to_str()
            .unwrap()
            .split(',')
            .any(|method| method.trim() == "POST")
    );
}

#[tokio::test]
async fn sse_only_valid_requests_are_rejected_before_dispatch_on_json_routes() {
    let cases = [
        ("GET", "/graphql?query=%7Bhello(name%3A%22Sheri%22)%7D", ""),
        ("POST", "/graphql", "{ hello(name: \"Sheri\") }"),
        ("POST", "/graphql", "mutation { hello(name: \"Sheri\") }"),
    ];
    let mut observed = Vec::new();
    for (method, uri, query) in cases {
        let (app, state) = contract_app();
        let body = if method == "GET" {
            Body::empty()
        } else {
            Body::from(json!({ "query": query }).to_string())
        };
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::ACCEPT, "text/event-stream")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        observed.push((
            status,
            state.dispatcher.calls.load(Ordering::SeqCst),
            state.dispatcher.sources.load(Ordering::SeqCst),
            body.is_empty(),
        ));
    }
    assert_eq!(
        observed,
        vec![(StatusCode::NOT_ACCEPTABLE, 0, 0, true); cases.len()],
        "GET query, POST query, and POST mutation must fail before execution"
    );
}

#[tokio::test]
async fn sse_only_transport_failures_preserve_status_with_an_empty_body() {
    let (app, state) = contract_app();
    for (content_type, body, expected_status) in [
        ("application/json", "{", StatusCode::BAD_REQUEST),
        ("application/json", "{}", StatusCode::UNPROCESSABLE_ENTITY),
        ("text/plain", "{}", StatusCode::UNSUPPORTED_MEDIA_TYPE),
    ] {
        let request = HttpRequest::builder()
            .method("POST")
            .uri("/graphql")
            .header(header::ACCEPT, "text/event-stream")
            .header(header::CONTENT_TYPE, content_type);
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected_status, "{content_type} {body}");
        assert!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(state.dispatcher.calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.dispatcher.sources.load(Ordering::SeqCst), 0);
    }
}
