use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    routing::{get, post},
};
use necrassrs::{Schema, Valid};
use necrassrs_axum::{GraphQLRequest, GraphQLResponse, graphiql_html, negotiate_response};

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
            post(graphql).route_layer(axum::middleware::from_fn(negotiate_response)),
        )
        .route("/graphql", get(|| async { graphiql_html("/graphql") }))
        .with_state(state)
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

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
    async fn graphiql_page_uses_the_existing_endpoint() {
        let response = app()
            .oneshot(Request::get("/graphql").body(Body::empty()).unwrap())
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
}
