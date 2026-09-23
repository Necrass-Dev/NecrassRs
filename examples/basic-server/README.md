# Basic server example

This is a Cargo consumer of `necrassrs`, `necrassrs-build`, and `necrassrs-axum`. It exposes the SDL in `schema/schema.graphql` through a user-owned Axum handler. `Query.hello` looks up names in a hardcoded list containing `Sheri` and `Margot`; matching is exact and case-sensitive, with no trimming.

## Build and run

From the repository root:

```sh
cargo run -p basic-server --locked
```

The server listens on `127.0.0.1:3000` and accepts JSON POST requests at `/graphql`. Cargo runs `build.rs` automatically; no separate generation command or installed CLI is needed.

The example's `[dependencies]` contain the runtime, Axum adapter, Axum, and Tokio. Its `[build-dependencies]` contain `necrassrs-build`. The NecrassRs dependencies use local paths and matching versions for this workspace. `serde_json` and `tower` are development dependencies used by the HTTP checks.

## Development UI

The example serves GraphiQL on GET `/graphql` by default. Open `http://127.0.0.1:3000/graphql`; the page sends requests to POST `/graphql`. The browser loads version-pinned assets from `esm.sh` and needs network access. Remove or gate the GET handler when deploying an application that should not expose the UI. See [Introspection and GraphiQL](../../docs/graphiql.md) for endpoint configuration and production introspection policy.

## Requests

Run these commands in a second terminal:

```sh
curl -sS http://127.0.0.1:3000/graphql \
  -H 'Content-Type: application/json' \
  --data '{"query":"{ hello(name: \"Sheri\") }"}'
```

```json
{"data":{"hello":"Hello, Sheri"}}
```

Variables use the same resolver and return the same response:

```sh
curl -sS http://127.0.0.1:3000/graphql \
  -H 'Content-Type: application/json' \
  --data '{"query":"query($name: String!) { hello(name: $name) }","variables":{"name":"Sheri"}}'
```

An unknown name produces an execution error with HTTP 200. For the query below, the response is:

```sh
curl -sS http://127.0.0.1:3000/graphql \
  -H 'Content-Type: application/json' \
  --data '{"query":"{ hello(name: \"Unknown\") }"}'
```

```json
{
  "errors": [{
    "message": "User \"Unknown\" was not found.",
    "locations": [{"line": 1, "column": 3}],
    "path": ["hello"],
    "extensions": {"code": "USER_NOT_FOUND"}
  }],
  "data": null
}
```

The server continues accepting requests after this error. Missing, null, or incompatible `name` inputs produce HTTP 422 with `errors` and no `data` entry. For example:

```sh
curl -i http://127.0.0.1:3000/graphql \
  -H 'Content-Type: application/json' \
  --data '{"query":"{ hello(name: null) }"}'
```

## Source ownership

| File | Owner and purpose |
| --- | --- |
| `schema/schema.graphql` | Application-owned public GraphQL contract |
| `build.rs` | Application-owned call to `necrassrs_build::build("schema")` |
| `src/main.rs` | Application-owned routing, Context construction, and server setup |
| `src/generated.rs` | Application-owned inclusion of `OUT_DIR/necrassrs.rs` |
| `src/resolvers.rs` | Build-managed resolver declarations with application-owned retained method bodies and unrelated code |
| `OUT_DIR/necrassrs.rs` | Disposable generated contracts, embedded SDL, and dispatch; never edit directly |

Cargo regenerates contracts when SDL changes. The build library synchronizes resolver declarations in `src/resolvers.rs`: retained method bodies are preserved, new fields receive `unimplemented!()` stubs, and deleted fields lose their methods. A rename is a deletion plus a new stub.

## Unimplemented resolver policy

Calling a selected `unimplemented!()` resolver must terminate the consumer process. A standalone consumer should set both profiles in its **application or workspace root** `Cargo.toml`:

```toml
[profile.dev]
panic = "abort"

[profile.release]
panic = "abort"
```

Cargo ignores profile settings in a non-root workspace member, so this setting belongs at the workspace root when the example is used there. It affects every panic in the configured profile. The `USER_NOT_FOUND` domain error above returns a GraphQL response and does not invoke this termination policy.

## Validation record

Checked on 2026-09-23 with macOS arm64 and Rust 1.96.1. The following commands passed from the repository root:

```sh
cargo build -p basic-server --locked --offline
cargo fmt --all -- --check
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
```

The workspace test run included all five basic-server tests, the build-library Cargo consumer tests, runtime execution tests, and Axum adapter tests. The checks below map [issue #1](https://github.com/Necrass-Dev/NecrassRs/issues/1) and [issue #6](https://github.com/Necrass-Dev/NecrassRs/issues/6) to observed results:

| Acceptance area | Passing check |
| --- | --- |
| Cargo generation and consumer compilation without a separate command | `cargo build -p basic-server`; the example includes generated contracts and its own resolver implementation |
| Hardcoded exact-name lookup | `basic-server` HTTP tests return exactly `Hello, Sheri` without errors; lowercase and padded names return `USER_NOT_FOUND` |
| Domain error, response path and location, root null propagation, and server availability | `basic-server::tests::unknown_name_returns_a_field_error_and_server_recovers` checks the exact response, alias path, variable error, and a later successful request |
| Literal and variable arguments | `basic-server::tests::literal_and_variable_requests_match` checks matching success responses; the error test checks both forms for unknown names |
| Missing, null, and incompatible arguments before resolver invocation | `basic-server::tests::invalid_inputs_are_request_errors` checks HTTP 422, errors, and omitted `data` for literals and variables; `necrassrs::execution::tests::invalid_variable_inputs_do_not_invoke_dispatcher` checks the invocation boundary directly |
| SDL additions, deletions, renames, and retained business logic | `necrassrs-build/tests/cargo_build.rs` builds disposable consumers from SDL and checks synchronization and unchanged method bodies; no duplicate synchronization test is added to this example |
| Borrowed Context and complete `Send` execution future | `necrassrs-build::codegen::test::generated_resolver_accepts_borrowed_context_and_returns_send_future` and `necrassrs::execution::tests::handwritten_dispatch_receives_coerced_arguments_and_borrowed_context` |
| Request Context isolation | `necrassrs::execution::tests::overlapping_requests_keep_their_context_values` and `necrassrs-axum/tests/http.rs::extracts_operation_variables_and_per_request_context`; this greeting example uses `()` per request |
| Unselected fields and selected unimplemented resolver termination | `necrassrs-build::codegen::test::selected_unimplemented_field_aborts_dev_and_release_consumers` checks a partial consumer in separate dev and release processes |
| Nullable list-item null propagation and other execution behavior | `necrassrs::execution::tests::null_non_null_item_nullifies_nullable_list` and related list, selection, and mutation tests |
| HTTP methods, extraction, response status, and body limits | `necrassrs-axum` HTTP tests and the basic-server HTTP tests |

Generated consumer contracts currently support `String!` field arguments and results. The list and mutation checks above belong to the runtime component and are not claims that this example can generate those types or roots. CLI initialization and packaged release builds are outside this example's scope.
