//! GraphQL request execution for SDL-first Rust applications.
//!
//! NecrassRs separates build-time contracts from runtime execution. The
//! `necrassrs-build` crate generates resolver traits, argument types, and a
//! dispatcher from SDL. Applications implement resolver bodies, own their
//! Context type, and pass a Context value to [`execute`] for each request.
//! HTTP integration is provided separately by `necrassrs-axum`.
//!
//! # Request flow
//!
//! 1. Parse your schema with [`Schema::parse_and_validate`], usually once at startup.
//! 2. Construct a [`Request`] with a document and optional variables/operation name.
//! 3. Call [`execute`] with the schema, dispatcher, and request Context.
//! 4. Serialize the returned [`Response`] or pass it to the HTTP adapter.
//!
//! Request validation errors omit `data`. Resolver failures become GraphQL
//! execution errors with a field path and source location; nullability determines
//! whether other data can be returned. Return [`ResolverError`] for application
//! failures. Panics are not converted into GraphQL errors.
//!
//! # Current scope
//!
//! Generated contracts support query fields returning `String!`, with no
//! arguments or `String!` arguments. The runtime also completes nullable/list
//! String results, but does not provide general output-object or scalar
//! completion. Subscriptions are rejected. Schema introspection is enabled by
//! default; use [`execute_with_options`] to disable it independently of any UI.

use apollo_compiler::response::ExecutionResponse;
pub use apollo_compiler::{
    Schema,
    response::{GraphQLError, JsonMap, JsonValue},
    validation::Valid,
};
use serde::Serialize;

mod execution;
mod input;

pub use execution::{Dispatcher, ExecutionOptions, FieldCoordinate, execute, execute_with_options};

/// An application failure returned by a resolver.
///
/// Execution preserves the message and extensions and adds the selected field's
/// location and alias-aware response path. Messages are exposed to clients.
///
/// ```
/// let error = necrassrs::ResolverError::new("User was not found")
///     .with_extension("code", "USER_NOT_FOUND");
/// assert_eq!(error.message(), "User was not found");
/// assert!(error.extensions().unwrap().contains_key("code"));
/// ```
pub struct ResolverError {
    message: String,
    extensions: Option<JsonMap>,
}

impl ResolverError {
    /// Creates an error with a client-visible message and no extensions.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            extensions: None,
        }
    }

    /// Adds a GraphQL error extension, replacing any value under the same key.
    pub fn with_extension(mut self, key: &str, value: impl Into<JsonValue>) -> Self {
        self.extensions
            .get_or_insert_default()
            .insert(key, value.into());

        self
    }

    /// Returns the client-visible message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns extensions, or `None` if none have been added.
    pub fn extensions(&self) -> Option<&JsonMap> {
        self.extensions.as_ref()
    }
}

/// A serializable GraphQL request-error or execution response.
///
/// Request errors omit `data`; execution responses include `data`, possibly
/// `null`, and any field errors. Normally obtained from [`execute`].
#[derive(Serialize)]
#[serde(transparent)]
pub struct Response(ResponseKind);

#[derive(Serialize)]
#[serde(untagged)]
enum ResponseKind {
    RequestError {
        errors: Vec<GraphQLError>,
        #[serde(skip)]
        syntax_error: bool,
    },
    Execution(ExecutionResponse),
}

impl Response {
    /// Whether this is a request-error response, with no `data` entry.
    ///
    /// An execution response with `data: null` still returns `false`.
    pub fn is_request_error(&self) -> bool {
        matches!(self.0, ResponseKind::RequestError { .. })
    }

    pub fn is_syntax_error(&self) -> bool {
        matches!(
            self.0,
            ResponseKind::RequestError {
                syntax_error: true,
                ..
            }
        )
    }

    /// Constructs a response for one error that prevented execution.
    pub fn request_error(error: GraphQLError) -> Self {
        Self::request_errors(vec![error])
    }

    /// Constructs a response without `data` from request errors.
    ///
    /// Callers should supply at least one error; this constructor does not validate it.
    pub fn request_errors(errors: Vec<GraphQLError>) -> Self {
        Self(ResponseKind::RequestError {
            errors,
            syntax_error: false,
        })
    }

    /// Constructs an execution response; `None` serializes as `data: null`.
    ///
    /// This constructor does not validate data against a schema.
    pub fn execution(data: Option<JsonMap>, errors: Vec<GraphQLError>) -> Self {
        Self(ResponseKind::Execution(ExecutionResponse { data, errors }))
    }
}

impl From<request::RequestError> for Response {
    fn from(error: request::RequestError) -> Self {
        Self(ResponseKind::RequestError {
            errors: error.errors,
            syntax_error: error.syntax_error,
        })
    }
}

mod request;

pub use request::Request;
