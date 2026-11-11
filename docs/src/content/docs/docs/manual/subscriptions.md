---
title: Subscriptions
description: Generate subscription resolvers, consume response streams, and handle event errors and cancellation.
---

A subscription creates a source stream once, then executes the selected GraphQL
fields for each event. Each event produces its own response. The application
supplies the event source and owns the request Context.

NecrassRs supports subscription execution and generated resolver contracts.
WebSocket and Server-Sent Events (SSE) transports are not included in the current
HTTP adapters. The example below runs in a console and finishes after two events;
it does not expose a GraphiQL subscription endpoint. See
[Integration](/docs/integration/#subscription-transports) for the transport boundary.

## Create a project

Install the CLI as described in [Installation](/docs/installation/#install-the-cli),
then create a project:

```sh
necrass init subscription-demo
cd subscription-demo
cargo add futures@0.3 serde_json@1
```

Keep the generated build script, dependency configuration, panic profiles, and
`src/generated.rs`. This example replaces the starter's HTTP entry point with a
console program so you can run a subscription without a transport.

These examples describe the current source checkout; published package versions
may differ.

## Declare the subscription

```graphql title="schema/schema.graphql"
type Query {
  hello(name: String!): String!
}

type Subscription {
  orderChanged(id: ID!): Order!
}

enum Status {
  PAID
  SHIPPED
}

type Order {
  id: ID!
  status: Status!
}
```

A Query root is still required. Each subscription operation selects one root
response key. Repeated selections with the same response key can merge, but
two different aliases count as two keys. Root introspection fields and root
`@skip` or `@include` selections are rejected, including through fragments.
Nested fields can use conditional directives normally.

Replace `schema/schema.graphql` with the SDL above, keeping the starter's
`Query.hello` field. Temporarily put this in `src/main.rs`, then run `cargo build`:

```rust
mod generated;
mod resolvers;

fn main() {}
```

The build adds editable subscription and Object resolver stubs in
`src/resolvers.rs`. The subscription uses the same marker and argument paths as
other fields:

- Root marker: `generated::resolvers::SubscriptionResolver<Context>`.
- Field marker: `generated::fields::Subscription::orderChanged`.
- Arguments: `generated::types::Subscription::orderChanged::Args`.

Subscription root fields implement `Subscriber::resolve`. Query, Mutation, and
Object fields implement `Resolver::resolve`. Existing naming and source ownership
rules apply; see [Organizing Resolver Files](/docs/resolver-files/).

## Implement the source and Object fields

Replace the generated stubs with this complete `src/resolvers.rs`:

```rust title="src/resolvers.rs"
use crate::generated::types;
use futures::stream;
use necrassrs::{Id, ResolverError, SourceEvent, SourceStream};

pub struct Query;
pub struct Subscription;
pub struct Order {
    pub id: Id,
    pub status: types::Status,
}

impl crate::generated::resolvers::QueryResolver<()> for Query {}
#[allow(non_snake_case)]
impl crate::generated::resolvers::SubscriptionResolver<()> for Subscription {}

impl necrassrs::Resolver<crate::generated::fields::Query::hello, ()> for Query {
    type Output = String;

    async fn resolve(
        &self,
        _: &(),
        args: types::Query::hello::Args,
    ) -> Result<Self::Output, ResolverError> {
        Ok(format!("Hello, {}", args.name))
    }
}

impl necrassrs::Subscriber<crate::generated::fields::Subscription::orderChanged, ()>
    for Subscription
{
    type Output = Order;

    async fn resolve<'a>(
        &'a self,
        _: &'a (),
        args: types::Subscription::orderChanged::Args,
    ) -> Result<SourceStream<'a, Self::Output>, ResolverError> {
        let events = [types::Status::PAID, types::Status::SHIPPED]
            .into_iter()
            .map(move |status| {
                SourceEvent::Value(Order {
                    id: args.id.clone(),
                    status,
                })
            });
        Ok(SourceStream::from_stream(stream::iter(events)))
    }
}

impl necrassrs::Resolver<crate::generated::fields::Order::id, ()> for Order {
    type Output = Id;

    async fn resolve(
        &self,
        _: &(),
        _: types::Order::id::Args,
    ) -> Result<Self::Output, ResolverError> {
        Ok(self.id.clone())
    }
}

impl necrassrs::Resolver<crate::generated::fields::Order::status, ()> for Order {
    type Output = crate::generated::types::Status;

    async fn resolve(
        &self,
        _: &(),
        _: types::Order::status::Args,
    ) -> Result<Self::Output, ResolverError> {
        Ok(self.status.clone())
    }
}
```

`Subscriber::Output` is one value of the SDL field: here it is `Order`, not a
stream type. Nullable, list, scalar, enum, and abstract results use the same
[type mappings](/docs/types/) as other resolver outputs. Selected Object fields
run against each event's own `Order` value.

`SourceStream::from_stream` accepts a `Send` stream of `SourceEvent<Output>`.
The input stream does not need to be `Sync`, `Unpin`, or `'static`; it may borrow
the subscriber and Context for `'a`. The subscriber is `Send + Sync`, Context
is `Sync`, and the source-creation future and event values are `Send`.
Borrowing a source does not enable borrowed Object outputs, which remain unsupported.

The example uses `()` as Context and a finite source. In an application, replace
them with your Context and event source. Construct and authorize that Context
before starting the subscription; the runtime does not create or refresh it
between events.

## Start the subscription and read responses

Replace `src/main.rs` with:

```rust title="src/main.rs"
use futures::{StreamExt, executor::block_on};
use necrassrs::{Request, Schema, subscribe};

mod generated;
mod resolvers;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    block_on(async {
        let schema = Schema::parse_and_validate(generated::SDL, "schema.graphql")
            .expect("generated SDL was validated during the build");
        let dispatcher = generated::dispatch::SchemaDispatcher::new(
            resolvers::Query,
            resolvers::Subscription,
        );
        let context = ();
        let request = Request::new(
            r#"subscription { changed: orderChanged(id: "42") { id status } }"#,
        );
        let mut responses = match subscribe(&schema, &request, &dispatcher, &context).await {
            Ok(stream) => stream,
            Err(response) => {
                eprintln!("{}", serde_json::to_string(&response)?);
                return Ok(());
            }
        };
        drop(request);

        while let Some(response) = responses.next().await {
            println!("{}", serde_json::to_string(&response?)?);
        }
        Ok(())
    })
}
```

Run `cargo run`. It prints two responses and exits:

```json
{"data":{"changed":{"id":"42","status":"PAID"}}}
{"data":{"changed":{"id":"42","status":"SHIPPED"}}}
```

The dispatcher constructor takes Query, Mutation, then Subscription roots,
omitting roots absent from the SDL. This schema has no Mutation root, so the
constructor takes only Query and Subscription.

Use `subscribe` for subscriptions and `execute` for queries and mutations.
`subscribe_with_options` accepts `ExecutionOptions` as its final argument.
Subscription root introspection remains invalid regardless of those options.

Startup validates the document and coerces variables once, then creates the
selected source once. `subscribe` returns `Ok(ResponseStream)` after creation
finishes, without consuming the first event. The original `Request` can then be
dropped, as above. Keep the schema, dispatcher, and Context alive while consuming
the response stream. The startup future is `Send`; `ResponseStream` is `Send`
and `Unpin` and yields `Result<Response, SubscriptionError>`.

## Handle errors and termination

| Situation                                                       | Result                                                         | Later events                  |
| --------------------------------------------------------------- | -------------------------------------------------------------- | ----------------------------- |
| Invalid request or source creation returns `Err(ResolverError)` | `subscribe` returns `Err(Response)` with errors and no `data`  | No response stream is created |
| Source yields `SourceEvent::Value(value)`                       | `Ok(Response)` after completing selected fields                | Continue                      |
| Source yields `SourceEvent::FieldError(error)`                  | `Ok(Response)` with a root-field execution error               | Continue                      |
| A selected field fails or a result cannot be serialized         | `Ok(Response)` with GraphQL errors and normal null propagation | Continue                      |
| Source yields `SourceEvent::SourceError(error)`                 | `Err(SubscriptionError::Source(error))`                        | Terminate                     |
| Source is exhausted                                             | `None`                                                         | Finished                      |

For example, use `FieldError` when one event cannot produce the root field's
value but the source can still deliver later events. Use `SourceError` when the
source has failed permanently. Errors returned while creating the source are
startup errors, including invalid custom scalar arguments.

An event response with `data: null` does **not** end the subscription. For this
schema, a root `FieldError` makes the event's data null because `orderChanged`
is non-null. The next successful event can still produce an ordinary response.
Execution errors retain field locations, alias-aware paths, and application
extensions.

`SubscriptionError::Internal` represents an exceptional runtime termination;
it is not an ordinary GraphQL field error. The API does not promise to catch
arbitrary application panics. A terminal stream error is yielded once, resources
are released, and later polls return `None`. Normal exhaustion also releases the
source without requiring an additional poll.

## Ordering, cancellation, and resource ownership

- Responses within one subscription preserve source-event order.
- Independent selected fields and list items within an event can make concurrent
  progress. Response ordering remains stable.
- Different subscriptions can make independent progress when polled concurrently.
- Dropping the response stream cancels its source and any in-progress field
  futures. Dropping startup before it finishes drops the source-creation future.
  Resources owned by those futures and streams are released through Rust's drop
  behavior.

The current executor completes each event before requesting the next and uses
no detached tasks or unbounded event queue for this flow. These are implementation
choices; future scheduling changes must account for resolver side effects,
buffering, ordering, and cancellation.

Your application owns any external producers and buffers: choose their
capacity, overflow policy, and cleanup behavior. Dropping a subscription does not
automatically stop a separately spawned producer. Reconnection, replay,
acknowledgements, and durable delivery also belong to the application and transport.
