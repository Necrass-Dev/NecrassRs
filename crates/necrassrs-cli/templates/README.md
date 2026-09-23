# NecrassRs starter

This project exposes a GraphQL endpoint at `http://127.0.0.1:3000/graphql` and a GraphiQL page at `http://127.0.0.1:3000/graphiql`.

Run the server:

```sh
cargo run
```

In another terminal, send a request:

```sh
curl -sS http://127.0.0.1:3000/graphql \
  -H 'Content-Type: application/json' \
  --data '{"query":"{ hello(name: \"Sheri\") }"}'
```

The response is `{"data":{"hello":"Hello, Sheri"}}`.

The GraphiQL page uses `/graphql` for schema introspection and queries. Its browser assets load from `esm.sh` and require network access. Remove or gate the `/graphiql` route in `src/main.rs` if the deployment should not expose the UI. Runtime introspection is enabled by default independently of the UI route; use `necrassrs::execute_with_options` with `ExecutionOptions { introspection: false }` to disable it for a handler.

Edit `schema/schema.graphql` to change the public API and `src/resolvers.rs` to implement its fields. Cargo runs `build.rs` automatically and keeps resolver declarations aligned with SDL. `src/generated.rs` includes disposable contracts from `OUT_DIR`. Generated argument and result types currently support `String!` fields. The NecrassRs dependencies use the framework's Git repository until packages are published.
