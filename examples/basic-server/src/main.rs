use std::sync::Arc;

use axum::{Router, extract::State, routing::post};
use necrassrs::{Schema, Valid};
use necrassrs_axum::{GraphQLRequest, GraphQLResponse};

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

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let schema = Schema::parse_and_validate(generated::SDL, "embedded.graphql").unwrap();
    let state = Arc::new(AppState {
        schema,
        dispatcher: generated::dispatch::SchemaDispatcher::new(resolvers::Query),
    });
    let app = Router::new()
        .route("/graphql", post(graphql))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await
}
