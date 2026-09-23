# NecrassRs starter

This project exposes a GraphQL endpoint at `http://127.0.0.1:3000/graphql`.

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

Edit `schema/schema.graphql` to change the public API and `src/resolvers.rs` to implement its fields. Cargo runs `build.rs` automatically and keeps resolver declarations aligned with SDL. `src/generated.rs` includes disposable contracts from `OUT_DIR`. Generated argument and result types currently support `String!` fields. The NecrassRs dependencies use the framework's Git repository until packages are published.
