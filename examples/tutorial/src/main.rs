use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::State,
    routing::{get, post},
};
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
}
