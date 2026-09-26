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
        .bind(("127.0.0.1", 3001))?
        .run()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{
        http::{StatusCode, header},
        test,
    };
    use serde_json::{Value, json};

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
    async fn built_in_graphiql_and_introspection_share_the_endpoint() {
        let app = test::init_service(App::new().configure(configure)).await;
        let request = test::TestRequest::get()
            .uri("/graphql")
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
}
