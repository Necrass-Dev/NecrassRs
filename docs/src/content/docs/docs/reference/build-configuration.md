---
title: Build Configuration
description: Cargo setup, schema discovery, generated modules, source synchronization, and build errors of necrassrs-build.
---

`necrassrs-build` validates SDL, generates Rust contracts, and synchronizes
resolver source during the Cargo build. Projects created by the
[CLI](/docs/cli/) include this setup.

This page describes the current source checkout. Use
[docs.rs](https://docs.rs/necrassrs-build/) for the API of a published version.

## Cargo setup

| Location                             | Entry                                                |
| ------------------------------------ | ---------------------------------------------------- |
| `[dependencies]`                     | `necrassrs` and the chosen HTTP adapter              |
| `[build-dependencies]`               | `necrassrs-build`                                    |
| `[profile.dev]`, `[profile.release]` | `panic = "abort"` in the application's root manifest |

The abort profiles make a call to an unimplemented resolver terminate the
process. They affect all panics, so return `ResolverError` for ordinary
application failures. Library Cargo profiles do not propagate to applications.

Call `build` from `build.rs` with the schema directory:

```rust title="build.rs"
fn main() -> Result<(), necrassrs_build::BuildError> {
    necrassrs_build::build("schema")
}
```

The CLI's `build.rs` also renders `BuildError::Codegen` as a source-annotated
report and exits with status 1.

Include the generated output in `src/generated.rs`, and declare `mod generated;`
and `mod resolvers;` in the application root:

```rust title="src/generated.rs"
include!(concat!(env!("OUT_DIR"), "/necrassrs.rs"));
```

## Schema discovery

`build(schema_dir)` reads every file with the `.graphql` extension under
`schema_dir`, recursively and sorted by path. Symbolic-link entries, including
linked directories, are skipped. A relative `schema_dir` is resolved against the
package root.

The build emits `cargo::rerun-if-changed` directives for the schema directory
and each discovered file. The schema must define a Query root.

## Outputs

| Path                   | Ownership                                                                |
| ---------------------- | ------------------------------------------------------------------------ |
| `OUT_DIR/necrassrs.rs` | Disposable; regenerated on every build                                   |
| `src/resolvers.rs`     | Synchronized with SDL; created when absent and written only when changed |

Resolver declarations in modules reachable from `src/resolvers.rs` are also
synchronized, and custom scalar declarations are written to
`src/resolvers/scalars.rs`. See [Organizing Resolver Files](/docs/resolver-files/)
for module discovery and source ownership, and
[Custom Scalars](/docs/custom-scalars/) for scalar regeneration.

## Generated modules

| Item                   | Contents                                                                  |
| ---------------------- | ------------------------------------------------------------------------- |
| `generated::SDL`       | The validated schema as SDL, for `Schema::parse_and_validate` at startup  |
| `generated::types`     | Enums, input objects, abstract result types, and per-field `Args` structs |
| `generated::fields`    | Field marker types, such as `fields::Query::hello`                        |
| `generated::resolvers` | Root and Object resolver traits, such as `QueryResolver<Context>`         |
| `generated::dispatch`  | `SchemaDispatcher`, which routes field calls to resolvers                 |

`SchemaDispatcher::new` takes the Query, Mutation, and Subscription root values
in that order, omitting roots absent from the SDL. See [Types](/docs/types/) for
the Rust type of each GraphQL type.

## Errors

`build` returns `BuildError`:

| Variant          | Cause                                                                  |
| ---------------- | ---------------------------------------------------------------------- |
| `Environment`    | `OUT_DIR` or `CARGO_MANIFEST_DIR` is missing or not Unicode            |
| `Io`             | A filesystem operation failed, including a rejected source destination |
| `Schema`         | SDL parsing or validation diagnostics                                  |
| `Codegen`        | A valid schema uses a feature that generation does not support         |
| `ResolverSource` | Resolver source could not be parsed or reconciled with the SDL         |

Invalid SDL and generation errors leave resolver source untouched. Generated
output is written before resolver synchronization, so a later error does not
imply that no files changed. Writes are not atomic; an interrupted write may
leave partial output. See
[Recover from synchronization errors](/docs/resolver-files/#recover-from-synchronization-errors)
for resolver source failures.
