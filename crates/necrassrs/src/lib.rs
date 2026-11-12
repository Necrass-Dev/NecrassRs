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
//! Generated query, mutation, and subscription contracts support built-in and custom
//! scalars, enum results, ordinary and OneOf input objects, nullable/list forms,
//! recursive input boxing, owned Object results, and abstract results. The runtime
//! preserves Object parents, list-item conversion errors, and nullability during
//! completion. Borrowed Object results remain unsupported. Schema introspection is
//! enabled by default; use [`execute_with_options`] to disable it independently of any UI.

use reed_compiler::response::ExecutionResponse;
pub use reed_compiler::{
    Schema,
    ast::{OperationType, Value as ScalarLiteral},
    response::{GraphQLError, JsonMap, JsonValue},
    validation::Valid,
};
use serde::Serialize;

mod execution;
mod input;
mod telemetry;

pub use execution::{
    Dispatcher, ExecutionOptions, FieldCoordinate, execute, execute_with_options, subscribe,
    subscribe_with_options,
};

/// A custom scalar conversion failure.
pub type ScalarError = ResolverError;

/// GraphQL literal arguments retained for generated custom scalar conversion.
pub type ScalarLiteralMap = std::collections::HashMap<String, ScalarLiteral>;

/// Converts values between GraphQL inputs, an application-owned scalar, and JSON output.
pub trait Scalar: Sized {
    /// Converts a borrowed GraphQL literal while preserving its literal kind.
    fn parse_literal(value: &ScalarLiteral) -> Result<Self, ScalarError>;

    /// Converts a borrowed variable input value.
    fn parse_value(value: &JsonValue) -> Result<Self, ScalarError>;

    /// Converts this scalar to a GraphQL response value.
    fn serialize(&self) -> Result<JsonValue, ScalarError>;
}

/// A resolved Object value that dispatches fields against its parent instance.
pub trait ResolvedObject<C>: Send + Sync {
    /// Returns the concrete GraphQL Object type name.
    fn type_name(&self) -> &'static str;

    /// Resolves one selected field against this Object value.
    ///
    /// The boxed future keeps recursive Object completion finite at the type level.
    fn resolve<'a>(
        &'a self,
        context: &'a C,
        coordinate: FieldCoordinate<'a>,
        arguments: &'a JsonMap,
    ) -> ::core::pin::Pin<
        Box<
            dyn ::core::future::Future<Output = Result<ResolvedValue<C>, ResolverError>>
                + Send
                + 'a,
        >,
    >;

    /// Resolves a field while retaining the original kinds of literal arguments.
    fn resolve_with_literals<'a>(
        &'a self,
        context: &'a C,
        coordinate: FieldCoordinate<'a>,
        arguments: &'a JsonMap,
        _argument_literals: &'a ScalarLiteralMap,
    ) -> ::core::pin::Pin<
        Box<
            dyn ::core::future::Future<Output = Result<ResolvedValue<C>, ResolverError>>
                + Send
                + 'a,
        >,
    > {
        self.resolve(context, coordinate, arguments)
    }
}

/// A resolved value tree retaining Object parents and conversion errors.
pub enum ResolvedValue<C = ()> {
    /// A JSON value, including explicit null and JSON arrays from custom dispatchers.
    Json(JsonValue),
    /// List items that may independently contain values or conversion errors.
    List(Vec<ResolvedValue<C>>),
    /// A concrete Object value used to resolve its selected subfields.
    Object(Box<dyn ResolvedObject<C>>),
    /// A conversion failure completed according to this position's nullability.
    Error(ResolverError),
}

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
#[derive(Debug)]
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
        #[serde(skip)]
        response_stream_required: bool,
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

    pub(crate) fn has_errors(&self) -> bool {
        match &self.0 {
            ResponseKind::RequestError { .. } => true,
            ResponseKind::Execution(response) => !response.errors.is_empty(),
        }
    }

    /// Whether request processing failed while parsing the GraphQL document.
    /// This classification is not serialized into the GraphQL response body.
    pub fn is_syntax_error(&self) -> bool {
        matches!(
            self.0,
            ResponseKind::RequestError {
                syntax_error: true,
                ..
            }
        )
    }

    /// Whether a validated subscription was passed to single-response execution.
    ///
    /// HTTP adapters use this to reject ordinary JSON delivery before creating a
    /// subscription source. Streaming transports should call [`subscribe`] instead.
    pub fn requires_response_stream(&self) -> bool {
        matches!(
            self.0,
            ResponseKind::RequestError {
                response_stream_required: true,
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
            response_stream_required: false,
        })
    }

    fn response_stream_required(error: GraphQLError) -> Self {
        Self(ResponseKind::RequestError {
            errors: vec![error],
            syntax_error: false,
            response_stream_required: true,
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
            response_stream_required: false,
        })
    }
}

mod request;

pub use request::Request;

/// The presence state of a nullable GraphQL argument or input object field.
pub enum GraphQLInput<T> {
    /// The value was omitted.
    Undefined,
    /// The value was explicitly provided as `null`.
    Null,
    /// A non-null value was provided.
    Value(T),
}

/// A GraphQL ID that preserves its supplied string representation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Id(String);

impl Id {
    /// Returns the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Id {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Id {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Identifies one generated GraphQL field and its argument type.
pub trait Field {
    /// Arguments passed to the field resolver after GraphQL coercion.
    type Args: Send;
}

/// Resolves a generated GraphQL field against an application-owned parent value.
///
/// Generated field marker types implement [`Field`]. Implement this trait for the
/// matching parent type and request Context to connect application logic to the
/// generated dispatcher.
pub trait Resolver<F, C>: Send + Sync
where
    F: Field,
    C: Sync,
{
    /// Rust value returned by this field before GraphQL value completion.
    type Output: Send;

    /// Resolves the field with its coerced arguments and request Context.
    fn resolve<'a>(
        &'a self,
        context: &'a C,
        args: F::Args,
    ) -> impl ::core::future::Future<Output = ::core::result::Result<Self::Output, ResolverError>>
    + Send
    + 'a;
}

/// Creates a source stream for a generated subscription root field.
pub trait Subscriber<F, C>: Send + Sync
where
    F: Field,
    C: Sync,
{
    /// One source event's value for this field.
    type Output: Send;

    /// Creates the source stream with coerced arguments and request Context.
    fn resolve<'a>(
        &'a self,
        context: &'a C,
        args: F::Args,
    ) -> impl ::core::future::Future<
        Output = ::core::result::Result<SourceStream<'a, Self::Output>, ResolverError>,
    > + Send
    + 'a;
}

/// One item produced by a subscription source.
pub enum SourceEvent<T> {
    /// A value to complete into one response event.
    Value(T),
    /// A field failure for this event; later events may still succeed.
    FieldError(ResolverError),
    /// A terminal source failure.
    SourceError(ResolverError),
}

/// A subscription source stream.
pub struct SourceStream<'a, T> {
    stream:
        Option<::core::pin::Pin<Box<dyn futures_util::Stream<Item = SourceEvent<T>> + Send + 'a>>>,
}

impl<'a, T: Send + 'a> SourceStream<'a, T> {
    /// Wraps a source stream without requiring it to be `Sync`, `Unpin`, or `'static`.
    pub fn from_stream(
        stream: impl futures_util::Stream<Item = SourceEvent<T>> + Send + 'a,
    ) -> Self {
        Self {
            stream: Some(Box::pin(stream)),
        }
    }

    /// Converts source values while preserving field and terminal source errors.
    pub fn map_values<U: Send + 'a>(
        self,
        mut map: impl FnMut(T) -> U + Send + 'a,
    ) -> SourceStream<'a, U> {
        SourceStream::from_stream(futures_util::StreamExt::map(
            self,
            move |event| match event {
                SourceEvent::Value(value) => SourceEvent::Value(map(value)),
                SourceEvent::FieldError(error) => SourceEvent::FieldError(error),
                SourceEvent::SourceError(error) => SourceEvent::SourceError(error),
            },
        ))
    }
}

impl<T> futures_util::Stream for SourceStream<'_, T> {
    type Item = SourceEvent<T>;

    fn poll_next(
        mut self: ::core::pin::Pin<&mut Self>,
        cx: &mut ::core::task::Context<'_>,
    ) -> ::core::task::Poll<Option<Self::Item>> {
        let Some(stream) = self.stream.as_mut() else {
            return ::core::task::Poll::Ready(None);
        };
        let next = stream.as_mut().poll_next(cx);
        if matches!(
            next,
            ::core::task::Poll::Ready(None | Some(SourceEvent::SourceError(_)))
        ) {
            self.stream = None;
        }
        next
    }
}

/// A terminal failure of a subscription response stream.
#[derive(Debug)]
pub enum SubscriptionError {
    /// The application source stream failed.
    Source(ResolverError),
    /// The runtime terminated exceptionally.
    Internal,
}

impl ::core::fmt::Display for SubscriptionError {
    fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
        match self {
            Self::Source(error) => formatter.write_str(error.message()),
            Self::Internal => formatter.write_str("Internal subscription error"),
        }
    }
}

impl std::error::Error for SubscriptionError {}

type BoxResponseStream<'a> = ::core::pin::Pin<
    Box<dyn futures_util::Stream<Item = Result<Response, SubscriptionError>> + Send + 'a>,
>;

/// Ordered GraphQL response events from a subscription.
pub struct ResponseStream<'a> {
    stream: Option<BoxResponseStream<'a>>,
}

impl<'a> ResponseStream<'a> {
    pub(crate) fn from_stream(
        stream: impl futures_util::Stream<Item = Result<Response, SubscriptionError>> + Send + 'a,
    ) -> Self {
        Self {
            stream: Some(Box::pin(stream)),
        }
    }
}

impl futures_util::Stream for ResponseStream<'_> {
    type Item = Result<Response, SubscriptionError>;

    fn poll_next(
        mut self: ::core::pin::Pin<&mut Self>,
        cx: &mut ::core::task::Context<'_>,
    ) -> ::core::task::Poll<Option<Self::Item>> {
        let Some(stream) = self.stream.as_mut() else {
            return ::core::task::Poll::Ready(None);
        };
        let next = stream.as_mut().poll_next(cx);
        if matches!(next, ::core::task::Poll::Ready(None | Some(Err(_)))) {
            self.stream = None;
        }
        next
    }
}

#[cfg(test)]
mod source_stream_tests {
    use std::{
        cell::Cell,
        sync::atomic::{AtomicUsize, Ordering},
        task::Poll,
    };

    use futures_util::{StreamExt, stream};

    use super::{ResolverError, SourceEvent, SourceStream};

    struct DropCount<'a>(&'a AtomicUsize);

    impl Drop for DropCount<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn send<T: Send>(value: T) -> T {
        value
    }

    fn unpin<T: Unpin>(_: &T) {}

    #[tokio::test]
    async fn accepts_borrowing_non_sync_non_unpin_streams() {
        let label = String::from("Sheri");
        let borrowed = label.as_str();
        let state = Cell::new(false);
        let input = stream::unfold(state, move |state| async move {
            if state.get() {
                None
            } else {
                state.set(true);
                Some((SourceEvent::Value(borrowed), state))
            }
        });
        let mut source = send(SourceStream::from_stream(input));
        unpin(&source);

        assert!(matches!(
            source.next().await,
            Some(SourceEvent::Value("Sheri"))
        ));
        assert!(source.next().await.is_none());
        assert!(source.next().await.is_none());
    }

    #[tokio::test]
    async fn releases_and_fuses_after_source_error() {
        let drops = AtomicUsize::new(0);
        let guard = DropCount(&drops);
        let mut emitted = false;
        let input = stream::poll_fn(move |_| {
            let _keep_alive = &guard;
            assert!(!emitted, "source polled after terminal failure");
            emitted = true;
            Poll::Ready(Some(SourceEvent::<()>::SourceError(ResolverError::new(
                "closed",
            ))))
        });
        let mut source = SourceStream::from_stream(input);

        assert!(matches!(
            source.next().await,
            Some(SourceEvent::SourceError(_))
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(source.next().await.is_none());
        assert!(source.next().await.is_none());
    }

    #[tokio::test]
    async fn maps_values_without_changing_error_kinds() {
        let mut source = SourceStream::from_stream(stream::iter([
            SourceEvent::Value(7),
            SourceEvent::FieldError(ResolverError::new("field")),
            SourceEvent::SourceError(ResolverError::new("source")),
        ]))
        .map_values(|value| value.to_string());

        assert!(matches!(source.next().await, Some(SourceEvent::Value(value)) if value == "7"));
        assert!(matches!(
            source.next().await,
            Some(SourceEvent::FieldError(error)) if error.message() == "field"
        ));
        assert!(matches!(
            source.next().await,
            Some(SourceEvent::SourceError(error)) if error.message() == "source"
        ));
        assert!(source.next().await.is_none());
    }
}

#[cfg(test)]
mod input_type_tests {
    use super::{GraphQLInput, Id};

    #[test]
    fn graphql_input_distinguishes_undefined_null_and_value() {
        assert!(matches!(
            GraphQLInput::<i32>::Undefined,
            GraphQLInput::Undefined
        ));
        assert!(matches!(GraphQLInput::<i32>::Null, GraphQLInput::Null));

        let input = GraphQLInput::Value(String::from("Sheri"));
        let GraphQLInput::Value(value) = input else {
            panic!("value input must preserve its payload");
        };
        assert_eq!(value, "Sheri");
    }

    #[test]
    fn id_preserves_supplied_string_contents() {
        let id = Id::from(String::from("001"));

        assert_eq!(id.as_str(), "001");
    }
}
