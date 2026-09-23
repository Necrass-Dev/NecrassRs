# Basic server example

This is a Cargo consumer of `necrassrs`, `necrassrs-build`, and `necrassrs-axum`. It exposes the SDL in `schema/schema.graphql` through a user-owned Axum handler. `Query.hello` looks up names in a hardcoded list containing `Sheri` and `Margot`; matching is exact and case-sensitive, with no trimming.

## Build and run

From the repository root:

```sh
cargo run -p basic-server --locked
```

The server listens on `127.0.0.1:3000` and accepts JSON POST requests at `/graphql`. Cargo runs `build.rs` automatically; no separate generation command or installed CLI is needed.

The example's `[dependencies]` contain the runtime, Axum adapter, Axum, and Tokio. Its `[build-dependencies]` contain `necrassrs-build`. The NecrassRs dependencies use local paths and matching versions for this workspace. `serde_json` and `tower` are development dependencies used by the HTTP checks.

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
