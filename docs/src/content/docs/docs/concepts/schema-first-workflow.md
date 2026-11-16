---
title: Schema-first Workflow
description: How SDL, Cargo builds, generated contracts, application code, and the runtime divide responsibility in a NecrassRs project.
---

In a NecrassRs project, GraphQL SDL is the source of truth for the public API.
Cargo builds turn that schema into Rust contracts, the application implements
them, and the runtime executes requests against them. This page explains how
those parts connect and who owns each one. See
[What is NecrassRs?](/docs/) for the motivation behind this approach.

## From schema to running server

```text
schema/**/*.graphql            written by you
        │  cargo build → build.rs → necrassrs-build
        ├─▶ OUT_DIR/necrassrs.rs     generated contracts, included as `generated`
        └─▶ src/resolvers.rs         resolver declarations synchronized with SDL
        │  rustc
application binary
        │  startup: Schema::parse_and_validate(generated::SDL)
        │  each request: execute(schema, request, dispatcher, Context)
GraphQL response
```

**Build time.** `necrassrs-build` runs inside the Cargo build script. It
validates the SDL, generates argument types, resolver traits, and a dispatcher,
and synchronizes resolver declarations in application source. The Rust compiler
then checks the application's resolver implementations against those contracts.
No separate generation command or installed CLI is involved; see
[Build Configuration](/docs/build-configuration/).

**Run time.** The application validates `generated::SDL` once at startup and
keeps the schema and dispatcher in shared state. For each request, it constructs
Context and calls the runtime, which prepares and executes the operation through
the dispatcher; see [Execution Model](/docs/execution-model/). Code generation
is not part of the request path, and the runtime does not depend on an HTTP
framework.

## Who owns what

| Part                                 | Owner             | Changes when                                  |
| ------------------------------------ | ----------------- | --------------------------------------------- |
| SDL under `schema/`                  | Application       | You edit the public contract                  |
| `OUT_DIR/necrassrs.rs`               | `necrassrs-build` | Every build regenerates it; never edit it     |
| Resolver declarations and signatures | SDL               | Builds synchronize them with the schema       |
| Resolver bodies and unrelated code   | Application       | You edit them; builds preserve them           |
| Context type and construction        | Application       | You define it and build one value per request |
| Routes, middleware, and shared state | Application       | You configure the HTTP framework              |
| Request and response conversion      | HTTP adapter      | See [Integration](/docs/integration/)         |
| Parsing, validation, and execution   | `necrassrs`       | See [Execution Model](/docs/execution-model/) |

Rust implementations are never synchronized back into SDL. To change the public
API, change the schema and build again.

## Context

Context carries request-specific values that resolvers need, such as the
authenticated user, a database pool handle, or per-request caches. NecrassRs
does not define its type or create it.

- **Type.** Generated resolver traits and stubs are generic over the Context
  type `C`, which must be `Sync`. A dispatcher uses one Context type for every
  root, so the application chooses a single type, such as `crate::Context`, for
  all resolvers.
- **Construction.** The application's handler builds the Context value for each
  request, typically from shared state and request data such as headers, before
  calling `execute` or `subscribe`.
- **Use.** Resolvers receive `&C`. The runtime borrows the value for the
  duration of execution and does not clone, refresh, or replace it, including
  between subscription events.

The [Tutorial](/docs/tutorial/) adds an application Context to a server, and the
[Personnel Management System](/docs/personnel-management/#6-protect-nullable-files-with-context)
uses it to protect a field.

## Evolving the schema

1. Edit the SDL.
2. Run `cargo build`. New fields receive `unimplemented!()` resolver stubs, and
   retained fields keep their bodies.
3. Review the source changes, implement new resolvers, and update retained
   bodies whose arguments or result types changed.

A deleted or renamed field loses its resolver only when that resolver is an
untouched generated stub. An edited implementation stops synchronization until
you remove or migrate it, so application code is never discarded silently. See
[Organizing Resolver Files](/docs/resolver-files/#understand-source-ownership)
for the exact rules and recovery steps.
