---
title: Introduction
description: Define a GraphQL API in SDL and build its Rust contracts with NecrassRs.
---

NecrassRs is an SDL-first GraphQL server framework for Rust. Your schema defines
the public API. Cargo generates Rust contracts and synchronizes resolver
declarations. You implement the resolver bodies.

:::caution[Under active development]
The generated API currently supports query fields with `String!` results and
built-in scalar, enum, and ordinary input object arguments, including nullable
and list forms. Design documents describe broader targets; they are not claims
of completed support.
:::

## Start with a working server

Follow the maintained [quick start in the repository](https://github.com/Necrass-Dev/NecrassRs#quick-start),
or explore the [Axum example](https://github.com/Necrass-Dev/NecrassRs/tree/main/examples/axum-server)
and [Actix Web example](https://github.com/Necrass-Dev/NecrassRs/tree/main/examples/actix-server).

## From schema to implementation

1. Define your API in `schema/**/*.graphql`.
2. Run `cargo build` to generate contracts and synchronize `src/resolvers.rs`.
3. Implement new resolver bodies and run the server.

Retained fields keep their existing method bodies. New fields receive
`unimplemented!()` stubs, which you must implement before calling them.
Deleting or renaming a field removes its old resolver method, including its body.
See [source synchronization](/docs/architecture/#73-partial-implementations-and-contract-changes)
for the full ownership contract.

## Explore the documentation

- [HTTP adapters](/docs/http/): Axum and Actix request/response behavior.
- [Introspection and GraphiQL](/docs/graphiql/): development UI and introspection settings.
- [Architecture](/docs/architecture/): crate responsibilities, source ownership, and planned work.
- [Specification requirements](/docs/specs/): conformance targets and acceptance cases.
- [Apollo Compiler patch](/docs/apollo-compiler/): the maintained dependency changes.
- [Default-cycle experiment](/docs/experiments/default-cycle-comparison/): algorithms and measurements.

These pages follow the source checkout from which this site is built. Dates and
implementation status in the migrated documents are retained; they do not describe
a versioned release. Rust API references are available locally with
`cargo doc --workspace --no-deps --locked --open`.
