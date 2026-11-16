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

The GraphiQL page uses `/graphql` for schema introspection and queries. Its browser assets load from `esm.sh` and require network access. Remove or gate the GET handler for `/graphql` in `src/main.rs` if the deployment should not expose the UI. Keep the POST handler for GraphQL requests. Runtime introspection is enabled by default independently of the UI route. Edit `src/options.rs` to change server-controlled execution settings, including introspection and the validation-work multiplier. The handlers already pass those options to `necrassrs::execute_with_options`. See the [`ExecutionOptions` API documentation](https://docs.rs/necrassrs/latest/necrassrs/struct.ExecutionOptions.html) for defaults and the validation limit. GraphQL requests cannot override these settings. For example, add
`validation_work_multiplier: 16,` before `..Default::default()` in
`src/options.rs` to allow more repeated validation work. Larger values weaken
this protection; rebuild and restart the service after editing the file.

Edit `schema/schema.graphql` to change the public API and `src/resolvers.rs` to implement its fields. Cargo runs `build.rs` automatically and keeps resolver declarations aligned with SDL. `src/generated.rs` includes disposable contracts from `OUT_DIR`. This starter begins with a `String!` field; generated contracts also support the input and composite result types documented by NecrassRs. The NecrassRs dependencies resolve from crates.io.

Both starters negotiate `Accept` for GraphQL requests without applying GraphQL response rules to the HTML page. The Axum starter uses `necrassrs_axum::negotiate_response` middleware; the Actix starter integrates negotiation into its GraphQL extractor and responder. Syntax errors return 400 and other GraphQL request errors 422. Error-free execution returns 200; execution errors return 294 for both JSON response types, including when `data` is null. See [GraphQL over HTTP](https://necrass.rs/docs/graphql-over-http/) for the status contract and compatibility guidance.
