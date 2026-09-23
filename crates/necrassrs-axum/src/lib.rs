//! Axum extraction and response conversion for GraphQL handlers.
//!
//! Mount application-owned handlers with `post`. Request bodies are JSON with
//! `query`, optional `variables`, and optional `operationName`. Axum accepts
//! `application/json` and JSON suffix media types. Its default 2 MiB body
//! limit applies; an application can set a different `DefaultBodyLimit`.
//!
//! Malformed JSON receives 400, invalid JSON fields receive 422, unsupported
//! content types receive 415, and oversized bodies receive 413. Route methods
//! other than POST receive Axum's 405. GraphQL request errors receive 422;
//! execution results, including domain errors, receive 200. GraphQL responses
//! use `application/json` regardless of `Accept`. Extractor failures use
//! Axum's rejection response.

use axum::{
    Json,
    extract::{FromRequest, Request as AxumRequest, rejection::JsonRejection},
    http::StatusCode,
    response::{Html, IntoResponse, Response as AxumResponse},
};
use necrassrs::{JsonMap, Request, Response};
use serde::Deserialize;

/// A GraphiQL page configured to send requests to an application-owned endpoint.
///
/// Mount this response on an application-chosen GET route. Browser assets load
/// from the version-pinned esm.sh URLs in the page and require network access.
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
pub struct GraphQLRequest(pub Request);

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
pub struct GraphQLResponse(pub Response);

impl IntoResponse for GraphQLResponse {
    fn into_response(self) -> AxumResponse {
        let status = if self.0.is_request_error() {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::OK
        };

        (status, Json(self.0)).into_response()
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
