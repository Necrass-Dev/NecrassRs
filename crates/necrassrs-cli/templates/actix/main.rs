use actix_web::{App, HttpServer, web};
use necrassrs::{Schema, Valid};
use necrassrs_actix::{GraphQLRequest, GraphQLResponse, graphiql_html};

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
            .route(web::get().to(|| async { graphiql_html("/graphql") })),
    );
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().configure(configure))
        .bind(("127.0.0.1", 3000))?
        .run()
        .await
}
