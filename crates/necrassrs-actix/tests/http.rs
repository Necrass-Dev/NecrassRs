use actix_web::{
    App, Error, HttpRequest,
    body::MessageBody,
    dev::{ServiceFactory, ServiceRequest, ServiceResponse},
    http::{Method, StatusCode, header},
    test, web,
};
use necrassrs::{
    Dispatcher, FieldCoordinate, JsonMap, JsonValue, ResolverError, Schema, Valid, execute,
};
use necrassrs_actix::{GraphQLRequest, GraphQLResponse, graphiql_html};
use serde_json::{Value, json};

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
    state: web::Data<AppState>,
    http_request: HttpRequest,
    GraphQLRequest(request): GraphQLRequest,
) -> GraphQLResponse {
    let context = Context {
        prefix: http_request
            .headers()
            .get("x-prefix")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("Hello")
            .to_owned(),
    };
    GraphQLResponse(execute(&state.schema, &request, &state.dispatcher, &context).await)
}

// Initialize with `let app = test::init_service(app()).await;` in an
// `#[actix_web::test]` function. The builder also accepts additional UI routes.
fn app() -> App<
    impl ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse,
        Error = Error,
        InitError = (),
    >,
> {
    app_at("/graphql")
}

fn app_at(
    endpoint: &str,
) -> App<
    impl ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse,
        Error = Error,
        InitError = (),
    >,
> {
    let schema = Schema::parse_and_validate(
        "type Query { hello(name: String!): String! nullableHello(name: String!): String }",
        "schema.graphql",
    )
    .unwrap();

    // Data supplies shared state to handlers without an additional Arc wrapper.
    App::new()
        .app_data(web::Data::new(AppState {
            schema,
            dispatcher: GreetingDispatcher,
        }))
        .service(web::resource(endpoint).route(web::post().to(graphql)))
}

// Add any extra headers to this builder, then pass `.to_request()` to
// `test::call_service(&app, request).await` and inspect with `response_parts`.
fn request(
    method: &str,
    content_type: Option<&str>,
    prefix: Option<&str>,
    body: impl Into<web::Bytes>,
) -> test::TestRequest {
    request_at("/graphql", method, content_type, prefix, body)
}

fn request_at(
    endpoint: &str,
    method: &str,
    content_type: Option<&str>,
    prefix: Option<&str>,
    body: impl Into<web::Bytes>,
) -> test::TestRequest {
    let mut request = test::TestRequest::default()
        .uri(endpoint)
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .insert_header((header::ACCEPT, "application/json"));
    if let Some(content_type) = content_type {
        request = request.insert_header((header::CONTENT_TYPE, content_type));
    }
    if let Some(prefix) = prefix {
        request = request.insert_header(("x-prefix", prefix));
    }
    request.set_payload(body)
}

async fn response_parts<B: MessageBody>(
    response: ServiceResponse<B>,
) -> (StatusCode, Option<String>, Value) {
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|value| value.to_str().unwrap().to_owned());
    let bytes = test::read_body(response).await;
    // Match the Axum fixture for non-JSON framework rejection responses.
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, content_type, body)
}

#[actix_web::test]
async fn extracts_operation_variables_and_per_request_context() {
    let app = test::init_service(app()).await;
    let body = json!({
        "query": "query First { hello(name: \"Skip\") } query Second($name: String!) { hello(name: $name) }",
        "operationName": "Second",
        "variables": { "name": "Sheri" }
    })
    .to_string();

    let req = request("POST", Some("application/json"), Some("Hi"), body.clone()).to_request();
    let response = test::call_service(&app, req).await;
    let (status, media_type, response_body) = response_parts(response).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(media_type.as_deref(), Some("application/json"));
    assert_eq!(response_body, json!({ "data": { "hello": "Hi, Sheri" } }));

    let req = request("POST", Some("application/json"), None, body).to_request();
    let response = test::call_service(&app, req).await;
    let (status, _, response_body) = response_parts(response).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response_body,
        json!({ "data": { "hello": "Hello, Sheri" } })
    );
}

#[actix_web::test]
async fn distinguishes_request_and_execution_errors_and_recovers() {
    let app = test::init_service(app()).await;

    let req = request(
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{ hello(name: 3) }" }).to_string(),
    )
    .to_request();
    let response = test::call_service(&app, req).await;
    let (status, _, response_body) = response_parts(response).await;

    // Existing Axum baseline; align this status with issue #18's HTTP contract.
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response_body.get("data").is_none());
    assert!(response_body.get("errors").is_some());

    let req = request(
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{ alias: hello(name: \"Unknown\") }" }).to_string(),
    )
    .to_request();
    let response = test::call_service(&app, req).await;
    let (status, _, response_body) = response_parts(response).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(response_body["data"], Value::Null);
    assert_eq!(
        response_body["errors"][0]["message"],
        "User \"Unknown\" was not found."
    );
    assert_eq!(response_body["errors"][0]["path"], json!(["alias"]));
    assert_eq!(
        response_body["errors"][0]["extensions"]["code"],
        "USER_NOT_FOUND"
    );

    let req = request(
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{ hello(name: \"Sheri\") }" }).to_string(),
    )
    .to_request();
    let response = test::call_service(&app, req).await;
    let (status, _, response_body) = response_parts(response).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response_body,
        json!({ "data": { "hello": "Hello, Sheri" } })
    );
}

#[actix_web::test]
async fn route_only_accepts_post() {
    // This checks the fixture's route configuration, not adapter-wide GET support.
    let app = test::init_service(app()).await;
    let req = request("GET", None, None, "").to_request();
    let response = test::call_service(&app, req).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers().get(header::ALLOW).unwrap(), "POST");
}

#[actix_web::test]
async fn schema_discovery_and_queries_share_the_existing_graphql_endpoint() {
    let app = test::init_service(app()).await;
    let req = request(
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{ __schema { queryType { name } } }" }).to_string(),
    )
    .to_request();
    let response = test::call_service(&app, req).await;
    let (status, _, response_body) = response_parts(response).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response_body,
        json!({ "data": { "__schema": { "queryType": { "name": "Query" } } } })
    );

    let req = request(
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{ hello(name: \"Sheri\") }" }).to_string(),
    )
    .to_request();
    let response = test::call_service(&app, req).await;
    let (status, _, response_body) = response_parts(response).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response_body,
        json!({ "data": { "hello": "Hello, Sheri" } })
    );
}

#[actix_web::test]
async fn built_in_graphiql_uses_the_application_route_and_configured_endpoint() {
    // The helper is built in; the application owns its UI route. The maintained
    // consumer example must register it by default, independently of this fixture.
    let without_ui = test::init_service(app_at("/api/graphql")).await;
    let req = test::TestRequest::get().uri("/graphiql").to_request();
    let response = test::call_service(&without_ui, req).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let app = test::init_service(app_at("/api/graphql").route(
        "/graphiql",
        web::get().to(|| async { graphiql_html("/api/graphql") }),
    ))
    .await;
    let req = test::TestRequest::get().uri("/graphiql").to_request();
    let page = test::call_service(&app, req).await;

    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(
        page.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );
    let html = test::read_body(page).await;
    let html = std::str::from_utf8(&html).unwrap();
    assert!(html.contains("data-endpoint=\"/api/graphql\""));
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
        let req = request_at(
            "/api/graphql",
            "POST",
            Some("application/json"),
            None,
            json!({ "query": query }).to_string(),
        )
        .to_request();
        let response = test::call_service(&app, req).await;
        let (status, _, response_body) = response_parts(response).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(response_body, expected);
    }
}

// HTTP acceptance cases informed by graphql-over-http revision 3903e680.
// These do not establish complete conformance or a policy for missing Accept.
#[actix_web::test]
async fn graphql_syntax_errors_return_400_without_data() {
    let app = test::init_service(app()).await;
    let req = request(
        "POST",
        Some("application/json"),
        None,
        json!({ "query": "{" }).to_string(),
    )
    .to_request();
    let response = test::call_service(&app, req).await;
    let (status, _, response_body) = response_parts(response).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(response_body.get("data").is_none());
    assert!(
        response_body["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
    );
}

#[actix_web::test]
async fn validation_operation_selection_and_variable_errors_return_422() {
    let app = test::init_service(app()).await;
    for body in [
        json!({ "query": "{ missing }" }),
        json!({ "query": "query A { hello(name: \"Sheri\") } query B { hello(name: \"Sheri\") }" }),
        json!({ "query": "query A { hello(name: \"Sheri\") }", "operationName": "Missing" }),
        json!({ "query": "query($name: String!) { hello(name: $name) }", "variables": {} }),
        json!({ "query": "query($name: String!) { hello(name: $name) }", "variables": { "name": null } }),
        json!({ "query": "query($name: String!) { hello(name: $name) }", "variables": { "name": 3 } }),
    ] {
        let req = request("POST", Some("application/json"), None, body.to_string()).to_request();
        let response = test::call_service(&app, req).await;
        let (status, _, response_body) = response_parts(response).await;
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

#[actix_web::test]
async fn partial_execution_errors_remain_http_200() {
    let app = test::init_service(app()).await;
    // Maintainer decision in #18: execution errors use 200, never 294.
    let body = json!({
        "query": "{ ok: hello(name: \"Sheri\") failed: nullableHello(name: \"Unknown\") }"
    })
    .to_string();
    let req = request("POST", Some("application/json"), None, body).to_request();
    let response = test::call_service(&app, req).await;
    let (status, _, response_body) = response_parts(response).await;
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

#[actix_web::test]
async fn negotiates_graphql_response_media_type() {
    let app = test::init_service(app()).await;
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
        let req = request(
            "POST",
            Some("application/json"),
            None,
            json!({ "query": "{ hello(name: \"Sheri\") }" }).to_string(),
        )
        .insert_header((header::ACCEPT, accept))
        .to_request();
        let response = test::call_service(&app, req).await;
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

#[actix_web::test]
async fn malformed_json_returns_400() {
    let app = test::init_service(app()).await;
    let req = request("POST", Some("application/json"), None, "{").to_request();
    let response = test::call_service(&app, req).await;
    let (status, _, _) = response_parts(response).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn invalid_request_fields_return_422() {
    let app = test::init_service(app()).await;
    for body in [
        json!({}),
        json!({ "query": 3 }),
        json!({ "query": "{ __typename }", "variables": [] }),
        json!({ "query": "{ __typename }", "operationName": 3 }),
    ] {
        let req = request("POST", Some("application/json"), None, body.to_string()).to_request();
        let response = test::call_service(&app, req).await;
        let (status, _, _) = response_parts(response).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
}

#[actix_web::test]
async fn unsupported_request_media_type_returns_415() {
    let app = test::init_service(app()).await;
    let req = request(
        "POST",
        Some("text/plain"),
        None,
        json!({ "query": "{ __typename }" }).to_string(),
    )
    .to_request();
    let response = test::call_service(&app, req).await;
    let (status, _, _) = response_parts(response).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}
