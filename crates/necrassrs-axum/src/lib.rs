//! Axum extraction and response conversion for GraphQL handlers.
//!
//! Applications own routing, shared schema/dispatcher state, and per-request
//! Context construction. [`GraphQLRequest`] extracts a runtime request;
//! [`GraphQLResponse`] converts the result of [`necrassrs::execute`] to HTTP.
//!
//! # Handler integration
//!
//! ```
//! use axum::{Router, extract::State, routing::post};
//! use necrassrs::{Dispatcher, Schema, Valid};
//! use necrassrs_axum::{GraphQLRequest, GraphQLResponse, negotiate_response};
//! use std::sync::Arc;
//!
//! struct App<D> { schema: Valid<Schema>, dispatcher: D }
//!
//! async fn graphql<D: Dispatcher<()> + Send + Sync + 'static>(
//!     State(state): State<Arc<App<D>>>,
//!     GraphQLRequest(request): GraphQLRequest,
//! ) -> GraphQLResponse {
//!     let context = (); // Replace with application-owned request Context.
//!     GraphQLResponse(necrassrs::execute(
//!         &state.schema, &request, &state.dispatcher, &context,
//!     ).await)
//! }
//!
//! fn app<D: Dispatcher<()> + Send + Sync + 'static>(state: Arc<App<D>>) -> Router {
//!     Router::new().route("/graphql", post(graphql::<D>).route_layer(axum::middleware::from_fn(negotiate_response))).with_state(state)
//! }
//! ```
//!
//! # HTTP behavior
//!
//! Mount application-owned handlers with `post`. Request bodies are JSON with
//! `query`, optional `variables`, and optional `operationName`. Axum accepts
//! `application/json` and JSON suffix media types. Its default 2 MiB body
//! limit applies; an application can set a different `DefaultBodyLimit`.
//!
//! Malformed JSON receives 400, invalid JSON fields receive 422, unsupported
//! content types receive 415, and oversized bodies receive 413. Route methods
//! other than POST receive Axum's 405. GraphQL syntax errors receive 400;
//! other GraphQL request errors receive 422. Execution results, including
//! domain errors, receive 200. Mount [`negotiate_response`] on GraphQL routes
//! to negotiate responses using `Accept`. Extractor failures retain Axum's
//! rejection response and are not relabeled as GraphQL responses.

use axum::{
    Json,
    extract::{FromRequest, Request as AxumRequest, rejection::JsonRejection},
    http::{StatusCode, header},
    middleware::Next,
    response::{Html, IntoResponse, Response as AxumResponse},
};
use necrassrs::{JsonMap, Request, Response};
use serde::Deserialize;

#[derive(Clone, Copy)]
struct RuntimeResponse;

/// Negotiates the response media type for an application-owned GraphQL route.
///
/// Missing `Accept` preserves JSON. Unsupported or malformed preferences return
/// 406 before invoking the handler. Only [`GraphQLResponse`] responses are
/// relabeled; HTML and framework errors retain their original content type.
/// Use `MethodRouter::route_layer` to preserve 405 responses for unsupported methods.
///
/// ```
/// use axum::{Router, middleware, routing::post};
/// use necrassrs_axum::negotiate_response;
/// let app: Router = Router::new().route(
///     "/graphql",
///     post(|| async { "replace with your GraphQL handler" })
///         .route_layer(middleware::from_fn(negotiate_response)),
/// );
/// ```
pub async fn negotiate_response(request: AxumRequest, next: Next) -> AxumResponse {
    let headers = request
        .headers()
        .get_all(header::ACCEPT)
        .iter()
        .map(|value| value.to_str())
        .collect::<Result<Vec<_>, _>>();
    let selected = headers.ok().and_then(necrassrs_http::response_media_type);
    let mut response = if let Some(media_type) = selected {
        let mut response = next.run(request).await;
        if response
            .extensions_mut()
            .remove::<RuntimeResponse>()
            .is_some()
        {
            // Keep non-2xx GraphQL results identifiable to legacy clients too.
            let media_type = if response.status().is_success() {
                media_type
            } else {
                necrassrs_http::GRAPHQL_JSON
            };
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, media_type.parse().unwrap());
        }
        response
    } else {
        StatusCode::NOT_ACCEPTABLE.into_response()
    };
    response
        .headers_mut()
        .append(header::VARY, "Accept".parse().unwrap());
    response
}

/// A GraphiQL page configured to send requests to an application-owned endpoint.
///
/// Mount this response on an application-chosen GET route. Browser assets load
/// from the version-pinned esm.sh URLs in the page and require network access.
/// The endpoint is escaped for its HTML attribute; this function does not create
/// the POST route or change runtime introspection settings. Gate or omit the UI
/// route when it should not be exposed in a deployment.
///
/// ```
/// use axum::{Router, routing::get};
/// use necrassrs_axum::graphiql_html;
/// let app: Router = Router::new()
///     .route("/playground", get(|| async { graphiql_html("/graphql") }));
/// ```
pub fn graphiql_html(endpoint_url: &str) -> Html<String> {
    let endpoint_url = endpoint_url
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    Html(include_str!("graphiql.html").replace("__NECRASSRS_ENDPOINT__", &endpoint_url))
}

/// A GraphQL request parsed from an Axum JSON request body.
///
/// Extracts `query`, optional `variables`, and optional `operationName`.
/// GraphQL document validation happens during runtime execution, not extraction.
/// This consumes the request body and should be the handler's last extractor.
/// Extraction failures use Axum's JSON rejection responses.
pub struct GraphQLRequest(
    /// Runtime request to pass to execution.
    pub Request,
);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestBody {
    query: String,
    variables: Option<JsonMap>,
    operation_name: Option<String>,
}

impl<S> FromRequest<S> for GraphQLRequest
where
    S: Send + Sync,
{
    type Rejection = JsonRejection;

    async fn from_request(req: AxumRequest, state: &S) -> Result<Self, Self::Rejection> {
        let Json(body) = Json::<RequestBody>::from_request(req, state).await?;
        let mut request = Request::new(body.query);

        if let Some(variables) = body.variables {
            request = request.with_variables(variables);
        }
        if let Some(operation_name) = body.operation_name {
            request = request.with_operation_name(operation_name);
        }

        Ok(Self(request))
    }
}

/// Converts a completed runtime response to an HTTP response.
///
/// Syntax errors receive HTTP 400, other request errors HTTP 422;
/// execution responses receive HTTP 200,
/// including responses with resolver errors or `data: null`. The content type
/// is `application/json` unless [`negotiate_response`] selects another type.
/// See the [crate documentation](crate) for extraction
/// errors, which follow a separate path.
pub struct GraphQLResponse(
    /// Completed runtime response.
    pub Response,
);

impl IntoResponse for GraphQLResponse {
    fn into_response(self) -> AxumResponse {
        let status = if self.0.is_syntax_error() {
            StatusCode::BAD_REQUEST
        } else if self.0.is_request_error() {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::OK
        };

        let mut response = (status, Json(self.0)).into_response();
        response.extensions_mut().insert(RuntimeResponse);
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::header};
    use necrassrs::GraphQLError;

    #[tokio::test]
    async fn rejects_invalid_json_requests() {
        for (content_type, body, expected) in [
            (None, "{}", StatusCode::UNSUPPORTED_MEDIA_TYPE),
            (Some("text/plain"), "{}", StatusCode::UNSUPPORTED_MEDIA_TYPE),
            (Some("application/json"), "{", StatusCode::BAD_REQUEST),
            (
                Some("application/json"),
                "{}",
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                Some("application/json"),
                r#"{"query":"{ hello }","variables":[]}"#,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
        ] {
            let mut request = AxumRequest::builder().uri("/graphql");
            if let Some(content_type) = content_type {
                request = request.header(header::CONTENT_TYPE, content_type);
            }
            let request = request.body(Body::from(body.to_owned())).unwrap();
            let rejection = GraphQLRequest::from_request(request, &())
                .await
                .err()
                .unwrap();
            assert_eq!(rejection.status(), expected, "{content_type:?} {body}");
        }

        let request = AxumRequest::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(vec![b'x'; 2 * 1024 * 1024 + 1]))
            .unwrap();
        let rejection = GraphQLRequest::from_request(request, &())
            .await
            .err()
            .unwrap();
        assert_eq!(rejection.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn maps_graphql_response_kinds_to_http_status() {
        let request_error = Response::request_error(GraphQLError {
            message: "Invalid request".into(),
            locations: vec![],
            path: vec![],
            extensions: JsonMap::new(),
        });
        assert_eq!(
            GraphQLResponse(request_error).into_response().status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );

        let execution = Response::execution(Some(JsonMap::new()), vec![]);
        assert_eq!(
            GraphQLResponse(execution).into_response().status(),
            StatusCode::OK
        );
    }

    #[test]
    fn graphiql_endpoint_is_escaped_in_html() {
        let html = graphiql_html("/graphql?x=\"<script>&'").0;
        assert!(html.contains("data-endpoint=\"/graphql?x=&quot;&lt;script&gt;&amp;&#39;\""));
        assert!(!html.contains("<script>&'"));
    }
}
