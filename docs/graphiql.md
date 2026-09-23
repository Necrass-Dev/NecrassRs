# Introspection and GraphiQL

GraphQL schema introspection is enabled by default in `necrassrs::execute`. It returns metadata from the validated Apollo schema through `__schema` and `__type`, including the supported generated schema's fields, arguments, and type references. `__typename` remains available as part of ordinary GraphQL execution.

An application may disable schema introspection for a handler with `execute_with_options`:

```rust
use necrassrs::{ExecutionOptions, execute_with_options};

let response = execute_with_options(
    &state.schema,
    &request,
    &state.dispatcher,
    &context,
    ExecutionOptions { introspection: false },
).await;
```

The application chooses this server-side setting. A disabled `__schema` or `__type` selection returns a GraphQL request error without invoking application resolvers. Disabling introspection does not replace authentication or field-level authorization.

## Optional Axum page

Register the page only where it should be available. The application still owns the route, GraphQL handler, and per-request Context:

```rust
use axum::{Router, routing::{get, post}};
use necrassrs_axum::graphiql_html;

let app = Router::new().route("/graphql", post(graphql));
let app = if development {
    app.route("/graphiql", get(|| async { graphiql_html("/graphql") }))
} else {
    app
};
```

`graphiql_html(endpoint_url)` sets the URL used for both schema introspection and ordinary GraphQL requests. A same-origin path such as `/graphql` needs no additional CORS setup. An absolute URL on another origin requires that endpoint to permit browser cross-origin requests. The helper only returns an HTML response; it does not create another GraphQL endpoint or server.

The page loads version-pinned GraphiQL, React, and GraphQL modules and the GraphiQL stylesheet from `https://esm.sh`. These assets are not bundled with the Rust crate, so the browser needs access to that CDN. A restrictive Content Security Policy must allow the relevant styles, scripts, and workers. The application may instead serve its own page if offline assets are required.

The basic-server example and projects created by `necrass init` register `/graphiql` by default. The page sends requests to each application's existing `/graphql` route. Remove or gate that route before deployment when the UI should be unavailable. This route choice is independent of introspection: the runtime allows introspection by default. An application that wants production introspection disabled must use `execute_with_options` in its GraphQL handler.
