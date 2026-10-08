---
title: "What is NecrassRS?"
description: An SDL-first GraphQL server framework that turns your schema into Rust resolver contracts.
---

NecrassRs is an SDL-first GraphQL server framework for Rust. You define your
public API in GraphQL Schema Definition Language (SDL), and NecrassRs turns that
schema into Rust types, resolver contracts, and dispatch code during Cargo builds.
You implement the resolver bodies with your application's logic.

The schema describes what clients can request. Rust implements how those requests
are fulfilled. Your application runs the web server, registers the GraphQL
endpoint, and supplies the Context that resolvers need for each request.

## Why NecrassRs?

GraphQL is most useful when frontend and backend developers can read the same
public API contract: what data is available, how it is shaped, and which
constraints apply. When that contract is inferred from Rust code, its source is
tied to the host language instead of being expressed in GraphQL's own schema
language.

NecrassRs makes GraphQL SDL the source of truth. Cargo builds turn that
language-independent contract into Rust types, resolver contracts, and dispatch
code. This reduces repetitive declarations and catches mismatches between the
schema and its Rust implementation at compile time, while leaving application
developers to write the business logic in ordinary Rust methods.

## Key features

- **SDL-first API definition.** Define fields, arguments, and types in GraphQL
  SDL. NecrassRs derives Rust contracts from that schema.
- **Cargo-integrated generation.** Your build script invokes `necrassrs-build`
  during ordinary Cargo builds. After project initialization, schema changes do
  not require a separate CLI generation command.
- **Resolver source synchronization.** Builds create new method stubs and preserve
  the bodies of retained methods. Deleted or renamed fields remove their old
  methods, including their bodies; unrelated application code is preserved.
- **Typed async resolvers.** Generated argument types and resolver traits connect
  GraphQL inputs and results to Rust types. The compiler checks implementations
  against those contracts, and async methods can borrow request Context.
- **Application-owned HTTP integration.** Adapters handle GraphQL request
  extraction and response conversion. Your application chooses routes,
  middleware, shared state, and how to construct Context. The execution core is
  independent of the HTTP framework.

:::caution[Under active development]
The generated API supports built-in scalar and enum query results, ordinary and
OneOf input objects, nullable and list forms, and recursive input layouts. Custom
scalars, composite results, and generated mutation/subscription routing remain
unsupported. These pages describe this source checkout, not an older resolved
Git dependency.
:::

## Namesake

NecrassRs is named after [Eblana Dublin (Necrass)](https://www.youtube.com/watch?v=X7vXSke1xFw), an operator in the game Arknights.

## Special Thanks

Special thanks to [XiNiHa](https://xiniha.dev/#about) for his contributions to NecrassRs: sparking the initial idea by highlighting gaps in Rust’s GraphQL ecosystem, introducing tonic as a valuable reference point, creating solid-relay, and continually helping shape the API through thoughtful discussions.

## Next steps

- [Tutorial](/docs/tutorial/): build a GraphQL web server and query it with
  GraphiQL or HTTP.
- [Types](/docs/types/): Rust type mappings, input presence, defaults, and
  generated input types.
