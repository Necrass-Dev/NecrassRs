use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::Poll,
};

use necrassrs::{
    Dispatcher, FieldCoordinate, JsonMap, JsonValue, ResolvedObject, ResolvedValue, ResolverError,
    ScalarError, ScalarLiteral, ScalarLiteralMap, Schema, SourceEvent, SourceStream, Valid,
};
use tracing::Instrument;

pub const SDL: &str = r#"
scalar Token
enum State { READY }
interface Named { value: Int! child: Node }
union Search = Node
type Node implements Named {
    value: Int!
    child: Node
    fail: Int
    fatal: Int!
    hold: Node
}
type Query {
    ping: Int!
    nullable: Int
    custom: Token!
    state: State!
    scalars: [Int!]!
    enums: [State!]!
    node: Node!
    nodes: [Node!]!
    named: Named!
    search: [Search!]!
    fail: Int
    bad: Int
    fatal: Int!
    broken: [Node]
    hold: Node
    echo(input: String!): String!
    token(input: Token!): Int!
    argument(required: Int! = 7): Node
    recovered: Int!
}
type Mutation { first: Node! second: Node! fatal: Int! }
type Subscription { events: Node! scalar: Int! }
"#;

#[derive(Default)]
pub struct Context {
    pub secret: String,
    pub block: bool,
    pub release: AtomicBool,
    pub starts: AtomicUsize,
    pub drops: AtomicUsize,
    pub source_drops: AtomicUsize,
    pub creation_drops: AtomicUsize,
    pub source_polls: AtomicUsize,
    pub order: Mutex<Vec<String>>,
}

pub struct DropCount<'a>(pub &'a AtomicUsize);
impl Drop for DropCount<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

pub fn schema() -> Valid<Schema> {
    Schema::parse_and_validate(SDL, "tracing.graphql").unwrap()
}

pub struct Query;
impl Dispatcher<Context> for Query {
    fn validate_custom_scalar_variable(
        &self,
        _: &str,
        value: &JsonValue,
    ) -> Result<(), ScalarError> {
        tracing::info_span!("app.scalar.variable").in_scope(|| {
            if value.as_str() == Some("reject-secret") {
                Err(failure())
            } else {
                Ok(())
            }
        })
    }

    fn validate_custom_scalar_literal(
        &self,
        _: &str,
        _: &ScalarLiteral,
    ) -> Result<(), ScalarError> {
        tracing::info_span!("app.scalar.literal").in_scope(|| Ok(()))
    }

    async fn resolve<'a>(
        &'a self,
        context: &'a Context,
        coordinate: FieldCoordinate<'a>,
        arguments: &'a JsonMap,
    ) -> Result<ResolvedValue<Context>, ResolverError> {
        async {
            tokio::task::yield_now().await;
            context.order.lock().unwrap().push(coordinate.field.into());
            let value = match coordinate.field {
                "ping" | "token" | "custom" => ResolvedValue::Json(42.into()),
                "nullable" => ResolvedValue::Json(JsonValue::Null),
                "state" => ResolvedValue::Json("READY".into()),
                "scalars" => ResolvedValue::List(vec![ResolvedValue::Json(1.into())]),
                "enums" => ResolvedValue::List(vec![ResolvedValue::Json("READY".into())]),
                "node" | "named" | "first" | "second" | "argument" => node(1),
                "nodes" | "search" => ResolvedValue::List(vec![node(1), node(2)]),
                "broken" => ResolvedValue::List(vec![node(1), ResolvedValue::Error(failure())]),
                "fail" | "fatal" => return Err(failure()),
                "bad" => ResolvedValue::Json("invalid-int-secret".into()),
                "hold" => {
                    wait(context).await;
                    node(1)
                }
                "echo" => {
                    assert_eq!(context.secret, "context-secret");
                    assert!(arguments.contains_key("input"));
                    ResolvedValue::Json("response-secret".into())
                }
                "recovered" => {
                    tracing::error!("recovered application failure");
                    ResolvedValue::Json(42.into())
                }
                field => panic!("unexpected root field {field}"),
            };
            Ok(value)
        }
        .instrument(tracing::info_span!("app.resolve", field = coordinate.field))
        .await
    }
}

pub fn failure() -> ResolverError {
    ResolverError::new("error-message-secret").with_extension("private", "extension-secret")
}

pub fn node(value: i32) -> ResolvedValue<Context> {
    ResolvedValue::Object(Box::new(Node(value)))
}
pub struct Node(pub i32);
impl ResolvedObject<Context> for Node {
    fn type_name(&self) -> &'static str {
        "Node"
    }

    fn resolve<'a>(
        &'a self,
        context: &'a Context,
        coordinate: FieldCoordinate<'a>,
        _: &'a JsonMap,
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedValue<Context>, ResolverError>> + Send + 'a>>
    {
        Box::pin(
            async move {
                match coordinate.field {
                    "value" => {
                        if context.block {
                            wait(context).await;
                        }
                        Ok(ResolvedValue::Json(self.0.into()))
                    }
                    "child" => Ok(node(self.0)),
                    "fail" | "fatal" => Err(failure()),
                    "hold" => {
                        wait(context).await;
                        Ok(node(self.0))
                    }
                    field => panic!("unexpected node field {field}"),
                }
            }
            .instrument(tracing::info_span!(
                "app.child",
                field = coordinate.field,
                item = self.0
            )),
        )
    }
}

pub async fn wait(context: &Context) {
    context.starts.fetch_add(1, Ordering::SeqCst);
    let _drop = DropCount(&context.drops);
    futures::future::poll_fn(|_| {
        if context.release.load(Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}

pub struct Events {
    pub values: Mutex<Option<Vec<SourceEvent<ResolvedValue<Context>>>>>,
    pub fail_start: bool,
    pub wait_start: bool,
    pub wait_source: bool,
}

impl Events {
    pub fn new(values: Vec<SourceEvent<ResolvedValue<Context>>>) -> Self {
        Self {
            values: Mutex::new(Some(values)),
            fail_start: false,
            wait_start: false,
            wait_source: false,
        }
    }
}

impl Dispatcher<Context> for Events {
    async fn resolve<'a>(
        &'a self,
        _: &'a Context,
        _: FieldCoordinate<'a>,
        _: &'a JsonMap,
    ) -> Result<ResolvedValue<Context>, ResolverError> {
        panic!("subscription must use source creation")
    }

    async fn resolve_event_stream<'stream, 'call>(
        &'stream self,
        context: &'stream Context,
        _: FieldCoordinate<'call>,
        _: &'call JsonMap,
        _: &'call ScalarLiteralMap,
    ) -> Result<SourceStream<'stream, ResolvedValue<Context>>, ResolverError>
    where
        'stream: 'call,
    {
        async {
            let _creation = DropCount(&context.creation_drops);
            if self.wait_start {
                wait(context).await;
            }
            if self.fail_start {
                return Err(failure());
            }
            let mut values = VecDeque::from(
                self.values
                    .lock()
                    .unwrap()
                    .take()
                    .expect("source created once"),
            );
            let guard = DropCount(&context.source_drops);
            Ok(SourceStream::from_stream(futures::stream::poll_fn(
                move |_| {
                    let _keep_alive = &guard;
                    context.source_polls.fetch_add(1, Ordering::SeqCst);
                    if self.wait_source && !context.release.load(Ordering::SeqCst) {
                        Poll::Pending
                    } else {
                        Poll::Ready(values.pop_front())
                    }
                },
            )))
        }
        .instrument(tracing::info_span!("app.source.create"))
        .await
    }
}
