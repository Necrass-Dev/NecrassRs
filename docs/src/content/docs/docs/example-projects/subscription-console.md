---
title: Subscription Console
description: Build a console program that declares a subscription, implements its event source, and prints each response.
---

This example declares an order-status subscription, implements a finite event
source, and prints each response in a console. It finishes after two events and
does not use an HTTP transport. See [Subscriptions](/docs/subscriptions/) for the
operation rules, resolver contracts, and error behavior used below.

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

A Query root is still required. Replace `schema/schema.graphql` with the SDL
above, keeping the starter's `Query.hello` field. Temporarily put this in
`src/main.rs`, then run `cargo build`:

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

`Subscriber::Output` is `Order`, one value of the SDL field, and selected Object
fields run against each event's own `Order` value.

The example uses `()` as Context and a finite source. In an application, replace
them with your Context and event source.

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

This schema has no Mutation root, so the dispatcher constructor takes only Query
and Subscription. The request is dropped after `subscribe` returns, while the
schema, dispatcher, and Context stay alive until the stream is consumed.
