---
title: Introduction
description: Define a GraphQL API in SDL and build its Rust contracts with NecrassRs.
---

NecrassRs is an SDL-first GraphQL server framework for Rust. Your schema defines
the public API. Cargo generates Rust contracts and synchronizes resolver
declarations. You implement the resolver bodies.

:::caution[Under active development]
The generated API supports built-in scalar and enum query results, ordinary and
OneOf input objects, nullable and list forms, and recursive input layouts. Custom
scalars, composite results, and generated mutation/subscription routing remain
unsupported. These pages describe this source checkout, not an older resolved
Git dependency.
:::

## Install the CLI

Install Rust and Cargo using the [official Rust installation guide](https://rust-lang.org/tools/install/).
Use a current stable toolchain. Then install the CLI from the Git repository:

```sh
cargo install --git https://github.com/Necrass-Dev/NecrassRs.git necrassrs-cli --locked
necrass --version
```

The Cargo package is named `necrassrs-cli`; the installed executable is named
`necrass`. If your shell cannot find it, ensure Cargo's binary directory is on
`PATH` (`~/.cargo/bin` on Unix or `%USERPROFILE%\.cargo\bin` on Windows).

## Create a project

Run the CLI without arguments in a terminal:

```sh
necrass
```

It asks for a project directory, Cargo package name, and Backend framework. The
default directory is `my-api`, and the package name defaults to the directory name.
Choose a supported backend framework. See [Integration](/docs/integration/) for the supported frameworks.

For scripts or when you already know the settings, use an explicit command:

```sh
necrass init my-api
```

This creates a project without asking questions. Use `--name <NAME>` to override
the Cargo package name and `--framework <FRAMEWORK>` to select a backend framework.

Without a path, `necrass init` uses the current directory. Targets must be new or empty;
existing projects, nonempty directories, and symbolic-link targets are rejected.

Use `necrass --help` and `necrass init --help` for command usage. Without a terminal,
supply an explicit command; bare `necrass` prints help and exits with an error.

The starter uses Git dependencies from the repository's default branch, including
the maintained Apollo Compiler patch. Cargo records the resolved revisions in the
new project's `Cargo.lock` on the first build. Keep that lockfile for repeatable
builds; later builds reuse it rather than following every new Git commit.

## Run the server and send a request

If you accepted the `my-api` directory or used the commands above:

```sh
cd my-api
cargo run
```

Both starters serve GraphQL on POST `/graphql` and GraphiQL on GET `/graphql` at
`http://127.0.0.1:3000`. Open [GraphiQL](http://127.0.0.1:3000/graphql) and run:

```graphql
{
  hello(name: "Sheri")
}
```

Or send the request from another terminal:

```sh
curl -sS http://127.0.0.1:3000/graphql \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/graphql-response+json' \
  --data '{"query":"{ hello(name: \"Sheri\") }"}'
```

The response is:

```json
{ "data": { "hello": "Hello, Sheri" } }
```

GraphiQL loads browser assets from `esm.sh` and needs network access. See
[Introspection and GraphiQL](/docs/graphiql/) for UI and introspection settings.

## From schema to implementation

1. Define your API in `schema/**/*.graphql`.
2. Run `cargo build` to generate contracts and synchronize `src/resolvers.rs`.
3. Implement new resolver bodies and run the server.

Retained fields keep their existing method bodies. New fields receive
`unimplemented!()` stubs, which you must implement before calling them.
Deleting or renaming a field removes its old resolver method, including its body.

After initialization, ordinary builds and runs use Cargo; the CLI is not needed
for schema changes. The starter configures `panic = "abort"` in development and
release builds, so calling an unimplemented resolver terminates the process.
Return `ResolverError` for ordinary application errors.
See [source synchronization](/docs/architecture/#73-partial-implementations-and-contract-changes)
for the full ownership contract.

## Explore the documentation

- [Generated types and inputs](/docs/types/): executable resolver examples, defaults, lists, OneOf, and recursion.
- [Integration](/docs/integration/): supported backend frameworks and request/response behavior.
- [Introspection and GraphiQL](/docs/graphiql/): development UI and introspection settings.
- [Architecture](/docs/architecture/): crate responsibilities, source ownership, and planned work.
- [Specification requirements](/docs/specs/): conformance targets and acceptance cases.
- [Apollo Compiler patch](/docs/apollo-compiler/): the maintained dependency changes.
- [Default-cycle experiment](/docs/experiments/default-cycle-comparison/): algorithms and measurements.

These pages follow the source checkout from which this site is built. Dates and
implementation status in the migrated documents are retained; they do not describe
a versioned release. Rust API references are available locally with
`cargo doc --workspace --no-deps --locked --open`.
