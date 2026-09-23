use apollo_compiler::response::ExecutionResponse;
pub use apollo_compiler::{
    Schema,
    response::{GraphQLError, JsonMap, JsonValue},
    validation::Valid,
};
use serde::Serialize;

mod execution;
mod input;

pub use execution::{Dispatcher, FieldCoordinate, execute};

pub struct ResolverError {
    message: String,
    extensions: Option<JsonMap>,
}

impl ResolverError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            extensions: None,
        }
    }

    pub fn with_extension(mut self, key: &str, value: impl Into<JsonValue>) -> Self {
        self.extensions
            .get_or_insert_default()
            .insert(key, value.into());

        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn extensions(&self) -> Option<&JsonMap> {
        self.extensions.as_ref()
    }
}

#[derive(Serialize)]
#[serde(transparent)]
pub struct Response(ResponseKind);

#[derive(Serialize)]
#[serde(untagged)]
enum ResponseKind {
    RequestError { errors: Vec<GraphQLError> },
    Execution(ExecutionResponse),
}

impl Response {
    /// Whether execution stopped before producing GraphQL data.
    pub fn is_request_error(&self) -> bool {
        matches!(self.0, ResponseKind::RequestError { .. })
    }

    pub fn request_error(error: GraphQLError) -> Self {
        Self::request_errors(vec![error])
    }

    pub fn request_errors(errors: Vec<GraphQLError>) -> Self {
        Self(ResponseKind::RequestError { errors })
    }

    pub fn execution(data: Option<JsonMap>, errors: Vec<GraphQLError>) -> Self {
        Self(ResponseKind::Execution(ExecutionResponse { data, errors }))
    }
}

mod request;

pub use request::Request;
