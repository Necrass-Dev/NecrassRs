//! Actix extraction and response conversion for application-owned GraphQL handlers.
//!
//! GET query parameters and POST JSON objects are supported. GET mutations
//! receive 405. JSON syntax errors receive 400, invalid request fields 422, unsupported
//! content types 415, and oversized bodies 413. Configure body limits through
//! `web::JsonConfig`. Custom JSON error handlers remain effective.
//! GraphQL syntax errors receive 400 and other request errors 422. All execution
//! results use 200, including partial data and root-null results with errors.
//! Responses negotiate `Accept`; missing headers preserve JSON and unsupported
//! preferences receive 406 before runtime execution. GraphiQL routes are owned
//! by the application and remain separate from response negotiation.

use actix_web::{
    FromRequest, HttpRequest, HttpResponse, Responder,
    body::BoxBody,
    error::{InternalError, JsonPayloadError},
    http::{Method, StatusCode, header},
    web::Html,
};
use necrassrs::{JsonMap, OperationType, Request, Response};
use serde::Deserialize;

/// Returns the built-in GraphiQL page for an application-selected endpoint.
///
/// Register this on an application-owned GET route. Assets load from pinned CDN
/// URLs; the endpoint is escaped for its HTML attribute. This helper neither
/// registers the GraphQL endpoint nor changes runtime introspection settings.
pub fn graphiql_html(endpoint_url: &str) -> Html {
    let endpoint_url = endpoint_url
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    Html::new(include_str!("graphiql.html").replace("__NECRASSRS_ENDPOINT__", &endpoint_url))
}

/// A GraphQL request parsed from GET parameters or an Actix JSON request body.
///
/// Extracts `query`, optional `variables`, `operationName`, and `extensions`.
/// GraphQL document validation happens during runtime execution, not extraction.
/// POST consumes the request body; other extractors must not consume it again.
/// Standard JSON extraction errors are mapped to the adapter's HTTP status codes.
/// Custom `JsonConfig` handlers returning other error types retain their behavior.
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
    #[serde(rename = "extensions")]
    _extensions: Option<JsonMap>,
    #[serde(flatten)]
    _unknown: JsonMap,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestQuery {
    query: String,
    operation_name: Option<String>,
    variables: Option<String>,
    extensions: Option<String>,
}

impl FromRequest for GraphQLRequest {
    type Error = actix_web::Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(
        req: &actix_web::HttpRequest,
        payload: &mut actix_web::dev::Payload,
    ) -> Self::Future {
        let negotiation = negotiated_media_types(req);
        let method = req.method().clone();
        if method == Method::GET {
            let query = actix_web::web::Query::<RequestQuery>::from_query(req.query_string())
                .map(|query| query.into_inner())
                .map_err(|error| error.to_string());
            return Box::pin(async move {
                let negotiation = negotiation?;
                let query =
                    query.map_err(|error| invalid_request(error, negotiation.json.is_none()))?;
                let body = RequestBody {
                    query: query.query,
                    variables: query
                        .variables
                        .as_deref()
                        .map(parse_optional_map)
                        .transpose()
                        .map_err(|error| {
                            invalid_request(error.to_string(), negotiation.json.is_none())
                        })?
                        .flatten(),
                    operation_name: query.operation_name.filter(|name| !name.is_empty()),
                    _extensions: query
                        .extensions
                        .as_deref()
                        .map(parse_optional_map)
                        .transpose()
                        .map_err(|error| {
                            invalid_request(error.to_string(), negotiation.json.is_none())
                        })?
                        .flatten(),
                    _unknown: JsonMap::new(),
                };
                finish_request(method, body)
            });
        }
        let extraction = actix_web::web::Json::<RequestBody>::from_request(req, payload);

        Box::pin(async move {
            let negotiation = negotiation?;
            let body = extraction
                .await
                .map_err(|error| {
                    let status = match error.as_error::<JsonPayloadError>() {
                        Some(JsonPayloadError::ContentType) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
                        Some(JsonPayloadError::Deserialize(cause)) if cause.is_data() => {
                            StatusCode::UNPROCESSABLE_ENTITY
                        }
                        Some(_) if negotiation.json.is_none() => {
                            error.as_response_error().status_code()
                        }
                        _ => return error,
                    };
                    let mut response = HttpResponse::build(status);
                    let response = if negotiation.json.is_none() {
                        response.finish()
                    } else {
                        response
                            .content_type("text/plain; charset=utf-8")
                            .body(error.to_string())
                    };
                    InternalError::from_response(error, response).into()
                })?
                .into_inner();
            finish_request(method, body)
        })
    }
}

fn finish_request(method: Method, body: RequestBody) -> Result<GraphQLRequest, actix_web::Error> {
    let RequestBody {
        query,
        variables,
        operation_name,
        _extensions: _,
        _unknown: _,
    } = body;
    let mut request = Request::new(query);
    if let Some(variables) = variables {
        request = request.with_variables(variables);
    }
    if let Some(operation_name) = operation_name {
        request = request.with_operation_name(operation_name);
    }
    if method == Method::GET && request.operation_type() == Some(OperationType::Mutation) {
        return Err(InternalError::from_response(
            "Mutations require POST",
            HttpResponse::MethodNotAllowed()
                .insert_header((header::ALLOW, "POST"))
                .finish(),
        )
        .into());
    }
    Ok(GraphQLRequest(request))
}

fn parse_optional_map(value: &str) -> Result<Option<JsonMap>, serde_json::Error> {
    if value.is_empty() {
        Ok(None)
    } else {
        serde_json::from_str(value)
    }
}

fn invalid_request(message: String, empty: bool) -> actix_web::Error {
    let mut response = HttpResponse::UnprocessableEntity();
    let response = if empty {
        response.finish()
    } else {
        response
            .content_type("text/plain; charset=utf-8")
            .body(message.clone())
    };
    InternalError::from_response(message, response).into()
}

/// Converts a runtime response to JSON with HTTP 200 for execution results,
/// including partial data and root-null results with errors.
///
/// Subscriptions passed to single-response execution receive 406.
/// GraphQL syntax errors receive 400; other request errors receive 422.
/// The response media type is selected from the request's `Accept` preferences.
pub struct GraphQLResponse(
    /// Completed runtime response.
    pub Response,
);

impl Responder for GraphQLResponse {
    type Body = BoxBody;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        let negotiation = match negotiated_media_types(req) {
            Ok(negotiation) => negotiation,
            Err(error) => return error.error_response(),
        };
        let status = if self.0.requires_response_stream() {
            StatusCode::NOT_ACCEPTABLE
        } else if self.0.is_syntax_error() {
            StatusCode::BAD_REQUEST
        } else if self.0.is_request_error() {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::OK
        };
        let Some(media_type) = negotiation.json else {
            return HttpResponse::build(status)
                .insert_header((header::VARY, "Accept"))
                .finish();
        };
        let media_type = if !status.is_success() && negotiation.legacy_json {
            necrassrs_http::GRAPHQL_JSON
        } else {
            media_type
        };
        HttpResponse::build(status)
            .insert_header((header::VARY, "Accept"))
            .content_type(media_type)
            .json(self.0)
    }
}

fn negotiated_media_types(
    req: &HttpRequest,
) -> Result<necrassrs_http::ResponseMediaTypes, actix_web::Error> {
    let headers = req
        .headers()
        .get_all(header::ACCEPT)
        .map(|value| value.to_str())
        .collect::<Result<Vec<_>, _>>();
    headers
        .ok()
        .and_then(necrassrs_http::response_media_types)
        .filter(|selected| selected.json.is_some() || selected.event_stream)
        .ok_or_else(|| {
            InternalError::from_response(
                "No acceptable GraphQL response media type",
                HttpResponse::NotAcceptable()
                    .insert_header((header::VARY, "Accept"))
                    .finish(),
            )
            .into()
        })
}
