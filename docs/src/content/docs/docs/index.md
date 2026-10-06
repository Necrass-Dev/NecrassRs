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

A GraphQL schema and its server implementation describe the same API from two
perspectives. Maintaining their declarations by hand creates repetitive work
whenever a field, argument, or type changes.

NecrassRs makes SDL the source of those declarations. Edit the schema and build
with Cargo to generate the Rust contracts and synchronize the resolver methods.
This keeps schema changes connected to the code you implement, while letting you
work with ordinary Rust methods and your application's existing server setup.

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

## Next steps

- [Tutorial](/docs/tutorial/): build a GraphQL web server and query it with
  GraphiQL or HTTP.
- [Types](/docs/types/): Rust type mappings, input presence, defaults, and
  generated input types.
- [Introspection and GraphiQL](/docs/graphiql/): configure schema discovery and
  the interactive query interface.
