# NecrassRs starter

This project serves GraphQL requests on POST `/graphql` and a GraphiQL page on GET `/graphql` at `http://127.0.0.1:3000`.

Run the server:

```sh
cargo run
```

In another terminal, send a request:

```sh
curl -sS http://127.0.0.1:3000/graphql \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/graphql-response+json' \
  --data '{"query":"{ hello(name: \"Sheri\") }"}'
```

The response is `{"data":{"hello":"Hello, Sheri"}}`.

The GraphiQL page uses `/graphql` for schema introspection and queries. Its browser assets load from `esm.sh` and require network access. Remove or gate the GET handler for `/graphql` in `src/main.rs` if the deployment should not expose the UI. Keep the POST handler for GraphQL requests. Runtime introspection is enabled by default independently of the UI route; use `necrassrs::execute_with_options` with `ExecutionOptions { introspection: false }` to disable it for a handler.

Edit `schema/schema.graphql` to change the public API and `src/resolvers.rs` to implement its fields. Cargo runs `build.rs` automatically and keeps resolver declarations aligned with SDL. `src/generated.rs` includes disposable contracts from `OUT_DIR`. Generated argument and result types currently support `String!` fields. The NecrassRs dependencies use the framework's Git repository until packages are published.

The GraphQL POST route includes `necrassrs_axum::negotiate_response` middleware. It negotiates `Accept` without applying GraphQL response rules to the HTML page. Syntax errors return 400, other GraphQL request errors 422, and execution results remain 200 even with errors. See the [HTTP adapter contract](https://github.com/Necrass-Dev/NecrassRs/blob/main/docs/http.md) for details.
