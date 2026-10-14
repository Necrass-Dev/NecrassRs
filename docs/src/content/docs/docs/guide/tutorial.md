---
title: Tutorial
description: Build a GraphQL web server, implement resolvers, and send queries and mutations with GraphiQL or HTTP.
---

In this tutorial, you will define a GraphQL schema, implement its generated Rust
resolvers, and run a web server. You will send queries and mutations using
GraphiQL and HTTP. The example introduces enums, input defaults, nullable values,
OneOf inputs, recursive input types, and shared in-memory state.

The completed Axum project is available in
[`examples/tutorial`](https://github.com/Necrass-Dev/NecrassRs/tree/main/examples/tutorial).

The source checkout supports built-in scalar and enum results, ordinary and OneOf
input objects, owned Object results, Interface and Union results, nullable and
list forms, recursive input and Object relationships, and generated query and
mutation routing. Custom scalars, borrowed Object results, and generated
subscription routing remain unsupported. These examples describe this checkout;
a previously resolved Git dependency must be updated before it contains these
changes. Use the maintained Apollo source replacement configured in the
repository's Cargo manifests.

## 1. Create a project

Follow the [installation guide](/docs/installation/#install-the-cli) to install the CLI, then
create a project:

```sh
necrass init my-api
cd my-api
```

Keep the generated Cargo manifest, build script, and `src/main.rs`. The starter
already registers a GraphQL HTTP endpoint and a GraphiQL page. You will replace
its schema and resolver implementation while keeping that server setup.

## 2. Define the schema

Replace `schema/schema.graphql` with this SDL, then run `cargo build`:

```graphql
enum Status {
  OPEN
  CLOSED
}

input Filter {
  status: Status = OPEN
  limit: Int
  next: Filter
  children: [Filter!]
}

input Locator @oneOf {
  id: ID
  name: String
}

type Query {
  statuses(filter: Filter = {}): [Status!]!
  label(locator: Locator!): String!
}
```

## 3. Implement the resolvers

After `cargo build`, open `src/resolvers.rs`. Cargo synchronizes the resolver
declarations with your schema and creates an empty root marker plus one field
resolver stub for each query field:

```rust
pub struct Query;

impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for Query {}

impl<C: Sync> necrassrs::Resolver<crate::generated::fields::Query::label, C> for Query {
    type Output = String;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::Query::label::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        ::core::unimplemented!()
    }
}

impl<C: Sync> necrassrs::Resolver<crate::generated::fields::Query::statuses, C>
    for Query
{
    type Output = Vec<crate::generated::types::Status>;

    async fn resolve(
        &self,
        _context: &C,
        _args: crate::generated::types::Query::statuses::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        ::core::unimplemented!()
    }
}
```

The marker identifies the Rust root type and Context used by dispatch. It stays
empty. Each field is a separate `Resolver<Field, C>` implementation with its
Rust result in `type Output`, so each field has an independent implementation
block. Each new `resolve` body starts with
`::core::unimplemented!()`. Rename `_args` to `args`, add the `GraphQLInput` import,
and replace the stub bodies with your resolver logic. Keep the generated
signatures and any unrelated application code.

For this example, the completed implementation is:

```rust
use necrassrs::GraphQLInput;

pub struct Query;

impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for Query {}

impl<C: Sync> necrassrs::Resolver<crate::generated::fields::Query::label, C> for Query {
    type Output = String;

    async fn resolve(
        &self,
        _context: &C,
        args: crate::generated::types::Query::label::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        match args.locator {
            crate::generated::types::Locator::id(id) => Ok(id.as_str().to_owned()),
            crate::generated::types::Locator::name(name) => Ok(name),
        }
    }
}

impl<C: Sync> necrassrs::Resolver<crate::generated::fields::Query::statuses, C>
    for Query
{
    type Output = Vec<crate::generated::types::Status>;

    async fn resolve(
        &self,
        _context: &C,
        args: crate::generated::types::Query::statuses::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        let GraphQLInput::Value(filter) = args.filter else {
            return Ok(Vec::new());
        };
        match filter.status {
            GraphQLInput::Value(status) => Ok(vec![status]),
            GraphQLInput::Undefined | GraphQLInput::Null => Ok(Vec::new()),
        }
    }
}
```

## 4. Run the web server

Start the generated server:

```sh
cargo run
```

Leave the server running in this terminal. The CLI starter listens on `127.0.0.1:3000` and serves
GraphQL requests through POST `/graphql`. GET `/graphql` opens GraphiQL, an
interactive interface for exploring the schema and sending queries.

### Query with GraphiQL

Open [http://127.0.0.1:3000/graphql](http://127.0.0.1:3000/graphql) in your browser.
Paste this query into the editor and click the execute button:

```graphql
{
  defaults: statuses
  nullFilter: statuses(filter: null)
  closed: statuses(filter: { status: CLOSED })
  label(locator: { id: 42 })
}
```

The response panel shows:

```json
{
  "data": {
    "defaults": ["OPEN"],
    "nullFilter": [],
    "closed": ["CLOSED"],
    "label": "42"
  }
}
```

Use GraphiQL's documentation explorer to browse the generated schema and its
arguments. The built-in page loads browser assets from a CDN and requires network
access. See [Introspection and GraphiQL](/docs/graphiql/) for UI configuration and
deployment settings.

### Query over HTTP

You can also send the same query from another terminal:

```sh
curl http://127.0.0.1:3000/graphql \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/graphql-response+json' \
  --data '{"query":"{ defaults: statuses nullFilter: statuses(filter: null) closed: statuses(filter: {status: CLOSED}) label(locator: {id: 42}) }"}'
```

The endpoint returns the same JSON response. See [Types](/docs/types/) for Rust
type mappings, `GraphQLInput`, defaults, OneOf inputs, and recursive inputs.

## 5. Add an in-memory mutation

This example stores a counter in the application Context shared by every
request. The Query and Mutation roots remain stateless. Stop the server with
Ctrl+C, append this definition to `schema/schema.graphql`, and run `cargo build`:

```graphql
type Mutation {
  increment(by: Int!): Int!
}
```

Cargo adds a `Mutation` root marker and a resolver stub for the new field. This
first build then reports Rust errors because `main.rs` still constructs a
query-only dispatcher; the changes below complete the new contract.

In `src/main.rs`, import `Mutex`, define the application Context, and store it
with the schema and dispatcher:

```rust
use std::sync::Mutex;

pub struct Context {
    counter: Mutex<i32>,
}

struct AppState {
    schema: Valid<Schema>,
    dispatcher: generated::dispatch::SchemaDispatcher<
        resolvers::Query,
        resolvers::Mutation,
    >,
    context: Context,
}
```

In `src/resolvers.rs`, leave both root types empty and make their marker
implementations use the application Context:

```rust
pub struct Query;
pub struct Mutation;

impl crate::generated::resolvers::QueryResolver<crate::Context> for Query {}
impl crate::generated::resolvers::MutationResolver<crate::Context> for Mutation {}
```

Update the two existing Query field implementations in the same way: replace
their generic `C` with `crate::Context`, and change `_context: &C` to
`_context: &crate::Context`. This gives every generated root the same Context
contract.

Replace the new resolver stub body with this implementation:

```rust
impl necrassrs::Resolver<
    crate::generated::fields::Mutation::increment,
    crate::Context,
> for Mutation
{
    type Output = i32;

    async fn resolve(
        &self,
        context: &crate::Context,
        args: crate::generated::types::Mutation::increment::Args,
    ) -> Result<Self::Output, necrassrs::ResolverError> {
        let mut counter = context
            .counter
            .lock()
            .map_err(|_| necrassrs::ResolverError::new("Counter is unavailable"))?;
        *counter = counter
            .checked_add(args.by)
            .ok_or_else(|| necrassrs::ResolverError::new("Counter overflow"))?;
        Ok(*counter)
    }
}
```

Pass the shared Context to `execute` instead of creating a unit Context for each
request:

```rust
necrassrs::execute(
    &state.schema,
    &request,
    &state.dispatcher,
    &state.context,
)
.await
```

For the Axum starter, create both stateless roots and initialize the Context
inside the existing `Arc<AppState>`:

```rust
let dispatcher = generated::dispatch::SchemaDispatcher::new(
    resolvers::Query,
    resolvers::Mutation,
);
let state = Arc::new(AppState {
    schema,
    dispatcher,
    context: Context {
        counter: Mutex::new(0),
    },
});
```

Axum shares this `Arc<AppState>` across requests. Actix calls its server factory
once per worker, so create one `web::Data<AppState>` before `HttpServer::new`
and clone that handle into every worker. Leave `configure` responsible only for
routes:

```rust
fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/graphql")
            .route(web::post().to(graphql))
            .route(web::get().to(|| async { graphiql_html("/graphql") })),
    );
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let schema = Schema::parse_and_validate(generated::SDL, "embedded.graphql").unwrap();
    let state = web::Data::new(AppState {
        schema,
        dispatcher: generated::dispatch::SchemaDispatcher::new(
            resolvers::Query,
            resolvers::Mutation,
        ),
        context: Context {
            counter: Mutex::new(0),
        },
    });

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .configure(configure)
    })
    .bind(("127.0.0.1", 3000))?
    .run()
    .await
}
```

All requests and Actix workers now share the same Context. A production server
can use the same structure with a database pool in place of the counter.
Restart the server and run this mutation in GraphiQL:

```graphql
mutation {
  first: increment(by: 2)
  second: increment(by: 3)
}
```

Top-level mutation fields complete serially in document order, so the response
shows each successive counter value:

```json
{
  "data": {
    "first": 2,
    "second": 5
  }
}
```

Running the mutation again continues from five. The counter returns to zero when
the server restarts. A real application can replace the `Mutex<i32>` with its
database or service state without changing the generated resolver contract.

## 6. Change the schema and rebuild

Stop the server with Ctrl+C, edit `schema/schema.graphql`, and run `cargo build`
again. Rebuilding adds new resolver stubs and retains existing resolver bodies.
Argument changes update generated `Args`; bodies that depend on removed or changed
fields need manual edits. Direct access to a removed argument stops synchronization
before resolver files are changed. For a retained field, Cargo updates `type Output` when
its result category, list wrapper, or nullability changes while preserving the
`resolve` body and parameter bindings. The body must still return the new Rust
type. New enum results use `crate::generated::types::<Enum>`. Deleting or
renaming a field removes an untouched generated stub; a user-edited resolver must
be removed or migrated explicitly. See [Organizing Resolver Files](/docs/resolver-files/)
to split implementations across modules and review synchronization safety.

Implement new resolver stubs, then restart the server with `cargo run`. Refresh
GraphiQL to explore the updated schema and execute your new queries. Continue with
the [Personnel Management System](/docs/personnel-management/) to add Object,
Interface, and Union results.
