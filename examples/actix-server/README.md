# Actix server example

This workspace consumer connects SDL, Cargo code generation, user resolvers, and an application-owned Actix Web handler. It uses `necrassrs`, `necrassrs-build`, and `necrassrs-actix` without an installed CLI.

## Build and run

From the repository root:

```sh
cargo run -p actix-server@0.1.0 --locked
```

The package version disambiguates this example from Actix Web's transitive `actix-server` dependency.

The server listens on `127.0.0.1:3001`, independently of the Axum example on port 3000. POST `/graphql` executes GraphQL; GET `/graphql` serves the built-in GraphiQL page by default.

Open `http://127.0.0.1:3001/graphql` or send a request:

```sh
curl -sS http://127.0.0.1:3001/graphql \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/graphql-response+json' \
  --data '{"query":"query Greeting($name: String!) { hello(name: $name) }","operationName":"Greeting","variables":{"name":"Sheri"}}'
```

The response is `{"data":{"hello":"Hello, Sheri"}}`. Names match `Sheri` and `Margot` exactly. An unknown name returns HTTP 200 with `data: null`, a field error, and the `USER_NOT_FOUND` extension. Subsequent requests remain available.

Actix handles response negotiation through the adapter's extractor and responder. No Axum middleware is needed. See the [HTTP contract](../../docs/http.md) for request errors, media types, and configurable body limits.

## Application ownership

`src/main.rs` owns the server, routes, schema state in `web::Data`, and Context construction. This greeting example uses `()` for its per-request Context. Each worker initializes its state through `configure`.

GraphiQL loads pinned assets from `esm.sh` and requires browser network access. Remove or gate the GET route when the UI should not be exposed. Introspection is enabled independently; use `execute_with_options` with `ExecutionOptions { introspection: false }` in the handler to disable it. See [Introspection and GraphiQL](../../docs/graphiql.md).

## SDL and regeneration

- `schema/schema.graphql` owns the public API.
- `build.rs` calls `necrassrs_build::build("schema")` during ordinary Cargo builds.
- `src/generated.rs` includes disposable contracts, embedded SDL, and dispatch from `OUT_DIR`.
- `src/resolvers.rs` contains synchronized declarations and application-owned resolver bodies.

Change the SDL and run `cargo build -p actix-server@0.1.0 --locked`. Retained resolver bodies are preserved; new fields receive `unimplemented!()` stubs and removed fields lose their methods. Implement a new stub before querying it. Generated contracts currently support `String!` arguments and results.

For a standalone application, set `panic = "abort"` in both `[profile.dev]` and `[profile.release]` at the application or workspace root so selecting an unimplemented resolver terminates the process. Cargo ignores profile settings in non-root members. Ordinary resolver errors use GraphQL responses instead of panics.

## Checks

```sh
cargo test -p actix-server@0.1.0 --locked
```

The example tests exercise generated resolvers with literal arguments, variables, operation selection, error propagation and recovery, and the default GraphiQL route with schema introspection. Adapter-level HTTP edge cases are tested in `necrassrs-actix`; these checks do not verify browser asset loading.
