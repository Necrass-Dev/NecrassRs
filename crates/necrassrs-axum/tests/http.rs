use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Request as HttpRequest, StatusCode, header},
    routing::post,
};
use necrassrs::{
    Dispatcher, FieldCoordinate, JsonMap, JsonValue, ResolverError, Schema, Valid, execute,
};
use necrassrs_axum::{GraphQLRequest, GraphQLResponse};
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
        assert_eq!(coordinate.field, "hello");
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
    let schema = Schema::parse_and_validate(
        "type Query { hello(name: String!): String! }",
        "schema.graphql",
    )
    .unwrap();
    Router::new()
        .route("/graphql", post(graphql))
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
    let mut builder = HttpRequest::builder().method(method).uri("/graphql");
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
    let (status, _, _) = send(&app, "GET", None, None, "").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}
