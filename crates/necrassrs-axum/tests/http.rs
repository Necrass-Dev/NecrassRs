use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Request as HttpRequest, StatusCode, header},
    routing::{get, post},
};
use necrassrs::{
    Dispatcher, FieldCoordinate, JsonMap, JsonValue, ResolverError, Schema, Valid, execute,
};
use necrassrs_axum::{GraphQLRequest, GraphQLResponse, graphiql_html};
use serde_json::{Value, json};
use tower::ServiceExt;

struct AppState {
    schema: Valid<Schema>,
    dispatcher: GreetingDispatcher,
}

struct GreetingDispatcher;

struct Context {
    prefix: String,
}

impl Dispatcher<Context> for GreetingDispatcher {
    async fn resolve<'a>(
        &'a self,
        context: &'a Context,
        coordinate: FieldCoordinate<'a>,
        arguments: &'a JsonMap,
    ) -> Result<JsonValue, ResolverError> {
        assert!(matches!(coordinate.field, "hello" | "nullableHello"));
        let name = arguments.get("name").and_then(JsonValue::as_str).unwrap();

        if name == "Unknown" {
            return Err(ResolverError::new("User \"Unknown\" was not found.")
                .with_extension("code", "USER_NOT_FOUND"));
        }

        Ok(format!("{}, {name}", context.prefix).into())
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
        .route(endpoint, post(graphql))
        .with_state(Arc::new(AppState {
            schema,
            dispatcher: GreetingDispatcher,
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
    let response = app
        .oneshot(HttpRequest::get("/graphql").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers().get(header::ALLOW).unwrap(), "POST");
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
