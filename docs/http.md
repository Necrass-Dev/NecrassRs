# HTTP adapters and response negotiation

Applications own routing, middleware, shared state, and per-request Context construction. `necrassrs-axum` and `necrassrs-actix` extract GraphQL requests and convert runtime responses. The execution core does not depend on either framework or on `necrassrs-http`.

`necrassrs-http` contains the response media-type selection shared by both adapters. It parses `Accept` preferences, applies quality and specificity rules, and handles explicit exclusions. It does not execute GraphQL, read request bodies, run a server, or own application routes. Applications normally depend on their adapter, which brings this package transitively.

## Axum

Attach `negotiate_response` using the GraphQL method router's `route_layer`. This applies negotiation only to registered methods and preserves 405 responses for unsupported methods regardless of `Accept`. It keeps the existing `GraphQLRequest(request)` and `GraphQLResponse(response)` handler API:

```rust
use axum::{Router, middleware, routing::{get, post}};
use necrassrs_axum::{graphiql_html, negotiate_response};

// `graphql` is the application's existing GraphQL handler.
let app = Router::new()
    .route("/graphql", post(graphql).route_layer(middleware::from_fn(negotiate_response)))
    .route("/graphql", get(|| async { graphiql_html("/graphql") }));
```

Axum's `IntoResponse` receives no request headers. The middleware reads `Accept` before invoking the handler and sets the content type only on responses produced by `GraphQLResponse`. Framework rejections and other responses retain their own formats. It appends `Vary: Accept` without replacing existing `Vary` values.

Without this middleware, `GraphQLResponse` preserves the existing `application/json` response behavior. The axum-server example and CLI template include the middleware. Mount it on the GraphQL route rather than wrapping GraphiQL HTML routes.

## Actix Web

`GraphQLRequest` implements Actix's `FromRequest`; `GraphQLResponse` implements `Responder`. The extractor checks acceptable response types before GraphQL execution, and the responder reads `Accept` directly from `HttpRequest`. No additional negotiation middleware is needed.

Use `web::Data` for application state and construct Context inside the handler. Configure JSON limits with `web::JsonConfig::limit`. Standard JSON extraction errors are mapped to the status codes below; custom JSON error handlers returning other error types retain their own responses.

The Actix adapter is under development. Its maintained consumer example and CLI framework-selection option are not delivered by these changes.

## Media types

- Send JSON request bodies with `Content-Type: application/json`.
- Prefer `Accept: application/graphql-response+json`; `application/json` remains supported for legacy clients.
- Quality weights (`q`), multiple header values, media-range wildcards, and specific `q=0` exclusions are respected. Equal quality prefers GraphQL JSON.
- Missing `Accept` preserves the JSON preference. An invalid header or no acceptable supported media type returns 406 before GraphQL execution.
- Negotiated runtime responses include `Vary: Accept`. Successful HTTP responses use the selected media type. Runtime request-error responses use `application/graphql-response+json`, including for legacy JSON clients, to distinguish them from intermediary HTTP errors.
- Non-GraphQL extraction failures retain framework error bodies; they are not labeled `application/graphql-response+json`.

For example:

```sh
curl -i http://127.0.0.1:3000/graphql \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/graphql-response+json' \
  --data '{"query":"{ hello(name: \"Sheri\") }"}'
```

## Status and error contract

| Condition | HTTP status | Response behavior |
| --- | --- | --- |
| Successful execution | 200 | GraphQL `data` |
| Execution errors, including partial data or `data: null` | 200 | GraphQL `data` and `errors` |
| GraphQL document syntax error | 400 | GraphQL `errors`, no `data` |
| GraphQL validation, operation selection, or variable coercion failure | 422 | GraphQL `errors`, no `data` |
| Malformed JSON body | 400 | Framework extraction error |
| Missing or incorrectly typed request fields | 422 | Framework extraction error |
| Missing or unsupported request Content-Type | 415 | Framework extraction error |
| Body exceeds configured JSON limit | 413 | Framework extraction error |
| No acceptable response media type or invalid Accept | 406 | Empty HTTP error response |
| Unsupported method on a POST-only resource | 405 | Framework route response, with `Allow: POST` |

The runtime distinguishes Apollo AST parsing failures from validation failures without interpreting diagnostic messages. `Response::is_syntax_error()` exposes that distinction to adapters; the classification is not serialized into GraphQL responses. Manually constructed `Response::request_error(s)` values remain general request errors.

NecrassRs deliberately does not adopt HTTP 294. Completed execution results use 200 even when errors propagate to a null root. See the [maintainer decision in #18](https://github.com/Necrass-Dev/NecrassRs/issues/18#issuecomment-5843384906).

## Scope and verification

These behaviors and acceptance cases are informed by [GraphQL-over-HTTP revision 3903e680](https://github.com/graphql/graphql-over-http/blob/3903e68045982cb6710880bf03527aa0e9331667/spec/GraphQLOverHTTP.md). This is a draft reference, not a claim of complete HTTP conformance or the final acceptance baseline for #18.

The current fixtures and starter expose GraphQL execution through POST. GET query extraction, GET mutation handling, subscription transports, and the remaining #18 conformance matrix are outside this implementation. A GraphiQL GET page is not a GraphQL GET execution endpoint. Body limits remain framework configuration, not a GraphQL-mandated byte count.

Run the focused checks with:

```sh
cargo test -p necrassrs-http -p necrassrs-axum -p necrassrs-actix --locked
```
