use std::sync::{Arc, Mutex};

use axum::{Router, extract::State, routing::get};
use necrassrs::{Schema, Valid};
use necrassrs_axum::{GraphQLRequest, GraphQLResponse, graphiql_html, negotiate_response};

mod generated;
mod resolvers;

pub struct Context {
    counter: Mutex<i32>,
}

struct AppState {
    schema: Valid<Schema>,
    dispatcher: generated::dispatch::SchemaDispatcher<resolvers::Query, resolvers::Mutation>,
    context: Context,
}

async fn graphql(
    State(state): State<Arc<AppState>>,
    GraphQLRequest(request): GraphQLRequest,
) -> GraphQLResponse {
    GraphQLResponse(
        necrassrs::execute(&state.schema, &request, &state.dispatcher, &state.context).await,
    )
}

fn app() -> Router {
    let schema = Schema::parse_and_validate(generated::SDL, "embedded.graphql").unwrap();
    let dispatcher =
        generated::dispatch::SchemaDispatcher::new(resolvers::Query, resolvers::Mutation);
    let state = Arc::new(AppState {
        schema,
        dispatcher,
        context: Context {
            counter: Mutex::new(0),
        },
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

    async fn execute(app: &Router, document: &str) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::post("/graphql")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "query": document }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn tutorial_queries_and_mutations_execute() {
        let app = app();
        let (status, response) = execute(
            &app,
            "{ defaults: statuses nullFilter: statuses(filter: null) closed: statuses(filter: { status: CLOSED }) label(locator: { id: 42 }) }",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            response,
            json!({
                "data": {
                    "defaults": ["OPEN"],
                    "nullFilter": [],
                    "closed": ["CLOSED"],
                    "label": "42"
                }
            })
        );

        let (status, response) = execute(
            &app,
            "mutation { first: increment(by: 2) second: increment(by: 3) }",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response, json!({ "data": { "first": 2, "second": 5 } }));

        let (status, response) = execute(&app, "mutation { increment(by: 1) }").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response, json!({ "data": { "increment": 6 } }));
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
