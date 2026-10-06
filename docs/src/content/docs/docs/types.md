---
title: Generated types and inputs
description: Built-in scalars, enums, input presence, lists, OneOf, and recursive inputs.
---

The source checkout supports built-in scalar and enum query results, including
nullable and list forms, and ordinary and OneOf input objects. Custom scalars,
composite results, and generated mutation/subscription routing remain unsupported.
These examples describe this checkout; a previously resolved Git dependency must
be updated before it contains these changes. Use the maintained Apollo source
replacement configured in the repository's Cargo manifests.

## Define the schema

Save this SDL under `schema/`, then run `cargo build`:

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

Cargo creates the declarations in `src/resolvers.rs`. Replace their stub bodies
with the following implementation. In an existing file, keep unrelated methods
and application state.

```rust
use crate::generated::{resolvers::QueryResolver, types};
use necrassrs::{GraphQLInput, ResolverError};

pub struct Query;

impl<C: Sync> QueryResolver<C> for Query {
    async fn statuses(
        &self,
        _context: &C,
        args: types::Query::statuses::Args,
    ) -> Result<Vec<types::Status>, ResolverError> {
        let GraphQLInput::Value(filter) = args.filter else {
            return Ok(Vec::new());
        };
        match filter.status {
            GraphQLInput::Value(status) => Ok(vec![status]),
            GraphQLInput::Undefined | GraphQLInput::Null => Ok(Vec::new()),
        }
    }

    async fn label(
        &self,
        _context: &C,
        args: types::Query::label::Args,
    ) -> Result<String, ResolverError> {
        match args.locator {
            types::Locator::id(id) => Ok(id.as_str().to_owned()),
            types::Locator::name(name) => Ok(name),
        }
    }
}
```

For a standalone executable, put the implementation above in `src/resolvers.rs`
and use this `src/main.rs`. The build script calls `necrassrs_build::build("schema")`;
the application depends on `necrassrs`, `serde_json`, and `futures`, with
`necrassrs-build` as a build dependency. No HTTP server is needed for this example.

```rust
mod generated {
    include!(concat!(env!("OUT_DIR"), "/necrassrs.rs"));
}
mod resolvers;

fn main() {
    let schema = necrassrs::Schema::parse_and_validate(generated::SDL, "schema.graphql")
        .unwrap();
    let dispatcher = generated::dispatch::SchemaDispatcher::new(resolvers::Query);
    let request = necrassrs::Request::new(
        "{ defaults: statuses nullFilter: statuses(filter: null) \
           closed: statuses(filter: {status: CLOSED}) label(locator: {id: 42}) }",
    );
    let response = futures::executor::block_on(necrassrs::execute(
        &schema, &request, &dispatcher, &(),
    ));
    let json = serde_json::to_value(response).unwrap();
    assert_eq!(json, serde_json::json!({"data": {
        "defaults": ["OPEN"], "nullFilter": [], "closed": ["CLOSED"], "label": "42"
    }}));
    println!("{json}");
}
```

The test suite compiles and executes these schema/resolver/main examples together.

## Input presence and defaults

Nullable arguments and ordinary nullable input fields use
`GraphQLInput<T> { Undefined, Null, Value(T) }` **after defaults and coercion**.
It describes the value the resolver receives, not the original request spelling.

| Input                                  | Resolver value                                          |
| -------------------------------------- | ------------------------------------------------------- |
| Omit `filter`                          | `Value(Filter { ... })`, from the argument default `{}` |
| Omit `status` inside a supplied filter | `Value(Status::OPEN)`, from the field default           |
| Supply `status: null`                  | `Null`; the default does not replace explicit null      |
| Omit `limit`                           | `Undefined`, because it has no default                  |
| Supply `limit: null`                   | `Null`                                                  |
| Supply `limit: 5`                      | `Value(5)`                                              |

A variable default applies when that variable is absent. An explicit null variable
does not activate its default. An absent argument variable permits an applicable
argument default. Unknown input fields are rejected before resolver dispatch.

## Rust mappings

| SDL                   | Rust                             |
| --------------------- | -------------------------------- |
| `Int!`                | `i32`                            |
| `Float!`              | `f64`                            |
| `String!`             | `String`                         |
| `Boolean!`            | `bool`                           |
| `ID!`                 | `necrassrs::Id`                  |
| `Status!`             | `generated::types::Status`       |
| Nullable input `Int`  | `GraphQLInput<i32>`              |
| Nullable result `Int` | `Option<i32>`                    |
| Input `[Int]`         | `GraphQLInput<Vec<Option<i32>>>` |
| Input `[Int]!`        | `Vec<Option<i32>>`               |
| Input `[Int!]`        | `GraphQLInput<Vec<i32>>`         |
| Input `[Int!]!`       | `Vec<i32>`                       |
| Result `[Int]`        | `Option<Vec<Option<i32>>>`       |
| Result `[Int]!`       | `Vec<Option<i32>>`               |
| Result `[Int!]`       | `Option<Vec<i32>>`               |
| Result `[Int!]!`      | `Vec<i32>`                       |

List items have no undefined state. Input singleton coercion applies recursively:
for `[[Int]]`, the input `3` becomes `[[3]]`. Enum literals are unquoted, as in
`status: CLOSED`; JSON enum variables use strings, such as `{"status":"CLOSED"}`.
Quoted GraphQL enum literals and unknown enum members are rejected.

`Id` preserves supplied string contents, including leading zeros, and serializes
results as strings. Integer inputs are accepted. `Int` enforces signed 32-bit
bounds. JSON variable numbers with an empty fractional part are treated as
integers; a GraphQL floating-point literal remains a different literal kind.
Non-finite Float results produce execution errors; nullable list items preserve
valid siblings and report the failed item's index.

## OneOf and recursion

`Locator` is generated as an enum with `id(Id)` and `name(String)` variants. The
selected payload has no `GraphQLInput` wrapper. Exactly one non-null entry is
required: `{id: "001"}` is valid, while `{}`, `{id: null}`, and
`{id: "001", name: "Sheri"}` are invalid. OneOf schema fields cannot be non-null
or have defaults. A variable used directly as a member value must have a
compatible non-null variable type.

In `Filter`, `next` uses `GraphQLInput<Box<Filter>>`; `children` uses
`GraphQLInput<Vec<Filter>>`. Lists already provide indirection. The generator
boxes nullable singular edges within recursive components, including mutual
recursion and OneOf payloads. Non-null and non-cyclic edges stay inline. Recursive
OneOf payloads use `Box<T>`, without another presence wrapper. GraphQL schema and
default-cycle validation are separate from this Rust layout analysis.

## Changing SDL

Rebuilding adds new method stubs and retains existing method bodies. Argument
changes update generated `Args`; bodies that depend on removed or changed fields
need manual edits. Retained methods keep their written return-type spelling, so
changing an SDL return type may also require editing the Rust return signature.
New enum-return methods use `crate::generated::types::<Enum>`. Deleting or renaming
a field removes its old method and body.
