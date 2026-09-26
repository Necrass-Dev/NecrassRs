use actix_web::{
    FromRequest, HttpRequest, HttpResponse, Responder,
    body::BoxBody,
    error::{InternalError, JsonPayloadError},
    http::{StatusCode, header},
    web::Html,
};
use necrassrs::{JsonMap, Request, Response};
use serde::Deserialize;

pub fn graphiql_html(endpoint_url: &str) -> Html {
    let endpoint_url = endpoint_url
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    Html::new(include_str!("graphiql.html").replace("__NECRASSRS_ENDPOINT__", &endpoint_url))
}

pub struct GraphQLRequest(
    pub Request,
);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestBody {
    query: String,
    variables: Option<JsonMap>,
    operation_name: Option<String>,
}

impl FromRequest for GraphQLRequest {
    type Error = actix_web::Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(
        req: &actix_web::HttpRequest,
        payload: &mut actix_web::dev::Payload,
    ) -> Self::Future {
        let negotiation = negotiated_media_type(req);
        let extraction = actix_web::web::Json::<RequestBody>::from_request(req, payload);

        Box::pin(async move {
            negotiation?;
            let body = extraction
                .await
                .map_err(|error| {
                    let status = match error.as_error::<JsonPayloadError>() {
                        Some(JsonPayloadError::ContentType) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
                        Some(JsonPayloadError::Deserialize(cause)) if cause.is_data() => {
                            StatusCode::UNPROCESSABLE_ENTITY
                        }
                        _ => return error,
                    };
                    let response = HttpResponse::build(status)
                        .content_type("text/plain; charset=utf-8")
                        .body(error.to_string());
                    InternalError::from_response(error, response).into()
                })?
                .into_inner();
            let mut request = Request::new(body.query);

            if let Some(variables) = body.variables {
                request = request.with_variables(variables);
            }
            if let Some(operation_name) = body.operation_name {
                request = request.with_operation_name(operation_name);
            }

            Ok(Self(request))
        })
    }
}

pub struct GraphQLResponse(
    pub Response,
);

impl Responder for GraphQLResponse {
    type Body = BoxBody;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        let media_type = match negotiated_media_type(req) {
            Ok(media_type) => media_type,
            Err(error) => return error.error_response(),
        };
        let status = if self.0.is_syntax_error() {
            StatusCode::BAD_REQUEST
        } else if self.0.is_request_error() {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::OK
        };
        let media_type = if status.is_success() {
            media_type
        } else {
            necrassrs_http::GRAPHQL_JSON
        };
        HttpResponse::build(status)
            .insert_header((header::VARY, "Accept"))
            .content_type(media_type)
            .json(self.0)
    }
}

fn negotiated_media_type(req: &HttpRequest) -> Result<&'static str, actix_web::Error> {
    let headers = req
        .headers()
        .get_all(header::ACCEPT)
        .map(|value| value.to_str())
        .collect::<Result<Vec<_>, _>>();
    headers
        .ok()
        .and_then(necrassrs_http::response_media_type)
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
