---
title: Tutorial
description: Build a GraphQL web server, implement resolvers, and send queries with GraphiQL or HTTP.
---

In this tutorial, you will define a GraphQL schema, implement its generated Rust
resolvers, and run a web server. You will send queries using GraphiQL and HTTP.
The example introduces enums, input defaults,
nullable values, OneOf inputs, and recursive input types.

The source checkout supports built-in scalar and enum query results, including
nullable and list forms, and ordinary and OneOf input objects. Custom scalars,
composite results, and generated mutation/subscription routing remain unsupported.
These examples describe this checkout; a previously resolved Git dependency must
be updated before it contains these changes. Use the maintained Apollo source
replacement configured in the repository's Cargo manifests.

## 1. Create a project

Follow the [installation guide](/docs/#install-the-cli) to install the CLI, then
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
declarations with your schema and creates these method stubs:

```rust
pub struct Query;

impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for Query {
    async fn r#label(
        &self,
        _context: &C,
        _args: crate::generated::types::Query::r#label::Args,
    ) -> ::core::result::Result<::std::string::String, ::necrassrs::ResolverError> {
        ::core::unimplemented!()
    }

    async fn r#statuses(
        &self,
        _context: &C,
        _args: crate::generated::types::Query::r#statuses::Args,
    ) -> ::core::result::Result<
        ::std::vec::Vec<crate::generated::types::r#Status>,
        ::necrassrs::ResolverError,
    > {
        ::core::unimplemented!()
    }
}
```

The signatures are generated for you; each new method starts with
`::core::unimplemented!()`. Rename `_args` to `args`, add the `GraphQLInput` import,
and replace the stub bodies with your resolver logic. Keep the generated
signatures and any unrelated application code.

For this example, the completed implementation is:

```rust
use necrassrs::GraphQLInput;

pub struct Query;

impl<C: Sync> crate::generated::resolvers::QueryResolver<C> for Query {
    async fn r#label(
        &self,
        _context: &C,
        args: crate::generated::types::Query::r#label::Args,
    ) -> ::core::result::Result<::std::string::String, ::necrassrs::ResolverError> {
        match args.locator {
            crate::generated::types::Locator::id(id) => Ok(id.as_str().to_owned()),
            crate::generated::types::Locator::name(name) => Ok(name),
        }
    }

    async fn r#statuses(
        &self,
        _context: &C,
        args: crate::generated::types::Query::r#statuses::Args,
    ) -> ::core::result::Result<
        ::std::vec::Vec<crate::generated::types::r#Status>,
        ::necrassrs::ResolverError,
    > {
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

Keep this terminal running. The CLI starter listens on `127.0.0.1:3000` and serves
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

## 5. Change the schema and rebuild

Stop the server with Ctrl+C, edit `schema/schema.graphql`, and run `cargo build`
again. Rebuilding adds new method stubs and retains existing method bodies. Argument
changes update generated `Args`; bodies that depend on removed or changed fields
need manual edits. Retained methods keep their written return-type spelling, so
changing an SDL return type may also require editing the Rust return signature.
New enum-return methods use `crate::generated::types::<Enum>`. Deleting or renaming
a field removes its old method and body.

Implement new resolver stubs, then restart the server with `cargo run`. Refresh
GraphiQL to explore the updated schema and execute your new queries.
