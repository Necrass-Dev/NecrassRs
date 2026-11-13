---
title: GraphQL over HTTP
description: HTTP request and response contracts, specification coverage, and transport limitations.
---

The maintained Axum and Actix Web adapters serve single-response GraphQL requests
over HTTP. This page describes the current source checkout against the
[GraphQL over HTTP September 2026 edition](https://http-spec.graphql.org/September2026/).
Published package versions may differ. This is a support contract, not a claim
that every normative clause or deployment configuration has been verified.

See [Integration](/docs/integration/) for route registration, middleware, and
application-owned Context construction.

## Responsibilities

| Layer                       | Responsibility                                                                                           |
| --------------------------- | -------------------------------------------------------------------------------------------------------- |
| `necrassrs` and Reed        | Parse and validate GraphQL, select operations, coerce inputs, execute resolvers, and complete responses. |
| `necrassrs-http`            | Select response representations from `Accept` preferences.                                               |
| Axum and Actix adapters     | Extract HTTP requests, reject unsupported delivery, and convert runtime responses to HTTP.               |
| Application and HTTP server | Own routes, authentication, Context, body and header limits, timeouts, TLS, and middleware.              |

HTTP is separate from the GraphQL execution contract. The runtime does not own a
server or depend on an HTTP framework. A proxy or framework can reject a request
before the GraphQL adapter receives it.

## Request support

The following table maps implemented behavior to the released specification.
`MUST`, `SHOULD`, and `MAY` describe the cited requirements, not test results.

| Specification                                                                                                                                                                      | Requirement                                                                     | Adapter behavior                                                                                                                                                                    |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [§3 Serialization](https://http-spec.graphql.org/September2026/#sec-Serialization-Format), [§4.4 POST](https://http-spec.graphql.org/September2026/#sec-POST)                      | MUST support UTF-8 JSON POST requests                                           | Both adapters accept `Content-Type: application/json`, including `charset=utf-8`.                                                                                                   |
| [§4.1 Request](https://http-spec.graphql.org/September2026/#sec-GraphQL-over-HTTP-Request), [§4.4.1 JSON encoding](https://http-spec.graphql.org/September2026/#sec-JSON-Encoding) | Required string `query`; typed optional fields; MUST ignore unknown properties  | Object envelopes are required. `operationName` is a string; `variables` and `extensions` are objects. Optional `null` values are treated as absent. Unknown properties are ignored. |
| [§4.3 GET](https://http-spec.graphql.org/September2026/#sec-GET)                                                                                                                   | GET support is optional; supported GET requests MUST use URL-encoded parameters | Queries are supported on GET routes in the examples and starters. `variables` and `extensions` are JSON-encoded URL parameters. Empty optional parameters are treated as absent.    |
| [§4.3 GET](https://http-spec.graphql.org/September2026/#sec-GET)                                                                                                                   | MUST NOT execute mutations via GET; 405 is recommended                          | Selected mutations are rejected before resolver dispatch with `405` and `Allow: POST`.                                                                                              |
| [§5.1 Body](https://http-spec.graphql.org/September2026/#sec-Body)                                                                                                                 | MUST support GraphQL JSON responses and respect supported `Accept` priorities   | Both JSON representations are supported; see [Media types](#media-types).                                                                                                           |
| [§5.4 Status codes](https://http-spec.graphql.org/September2026/#sec-Status-Codes)                                                                                                 | Distinguish request errors from execution results                               | Request errors use the applicable 4xx status. Execution results use 200 or 294 as described below.                                                                                  |

The adapters validate the shape of `extensions` but do not pass its contents into
the runtime `Request`. Accepting an extension object does not implement persisted
documents or other extension protocols. JSON array envelopes are rejected;
request batching is not provided.

## Media types

- Send JSON bodies with `Content-Type: application/json`.
- Prefer `Accept: application/graphql-response+json`. Legacy `application/json`
  responses remain supported.
- Quality weights (`q`), repeated header values, wildcards, and explicit `q=0`
  exclusions are respected. Equal quality prefers GraphQL JSON.
- Missing `Accept` selects legacy JSON. Malformed preferences or no acceptable
  supported representation produce `406` before GraphQL execution.
- Runtime responses include `Vary: Accept`. Successful responses, including 294,
  retain the selected JSON media type.
- A non-2xx GraphQL response switches to GraphQL JSON when the client accepts
  legacy JSON but not GraphQL JSON, following §5.1. If both are acceptable and
  legacy JSON has higher priority, that preference remains effective.
- Framework extraction failures retain their own error representation when
  ordinary JSON is acceptable. They are not relabeled as GraphQL responses.

Axum requires the `negotiate_response` route middleware for negotiation. Without
it, `GraphQLResponse` uses `application/json`; its status classification is
unchanged. Actix's responder reads the request headers directly. See the
[framework setup instructions](/docs/integration/#using-an-adapter).

The adapters serialize UTF-8 JSON but do not explicitly append `charset=utf-8`
to GraphQL response content types. The specification defaults these media types
to UTF-8 and recommends explicitly indicating the encoding in responses.

## Status and error contract

| Condition                                                             | HTTP status           | Response behavior                                 |
| --------------------------------------------------------------------- | --------------------- | ------------------------------------------------- |
| Error-free execution, including an empty data object                  | 200                   | GraphQL `data`, no `errors`                       |
| Execution errors with partial data or `data: null`                    | 294                   | GraphQL `data` and `errors`                       |
| GraphQL document syntax error                                         | 400                   | GraphQL `errors`, no `data`                       |
| GraphQL validation, operation selection, or variable coercion failure | 422                   | GraphQL `errors`, no `data`                       |
| Malformed JSON body                                                   | 400                   | Framework extraction error                        |
| Missing or incorrectly typed request fields, non-object envelopes     | 422                   | Framework extraction error                        |
| Missing or unsupported request Content-Type                           | 415                   | Framework extraction error                        |
| Body exceeds configured JSON limit                                    | 413                   | Framework extraction error                        |
| URI exceeds the parser limit in the HTTP/1.1 regression case          | Axum: 414; Actix: 400 | Framework parser error                            |
| No acceptable response representation or malformed Accept             | 406                   | Empty HTTP error response                         |
| Selected mutation over GET                                            | 405                   | Empty response, with `Allow: POST`                |
| Subscription sent to single-response JSON execution                   | 406                   | GraphQL `errors`, no `data`; no source is created |

Request errors omit `data`. A completed execution can contain `data: null` after
non-null propagation and still uses 294, not a request-error status. Resolver
messages, locations, alias-aware paths, and error extensions are preserved.

### HTTP 294 and compatibility

Both `application/graphql-response+json` and `application/json` use 294 for
execution errors, including when `Accept` is absent. This replaces the previous
200 behavior; error-free responses remain 200. There is no legacy status mode.

This follows §5.4's explicit 294 recommendation and §5.1's legacy-client rule:
process the request normally and change the media label for 2xx responses. The
released document is internally inconsistent: §5.4.1 still describes 200 for
execution errors, and the non-normative §6.1 recommends 294 only alongside
GraphQL JSON. NecrassRs follows the normative status and legacy-labeling rules
for both JSON representations.

The specification defines 294 as a custom code, not an IETF-standardized status.
Its [partial-success discussion](https://http-spec.graphql.org/September2026/#sec-Partial-success)
explains the observability motivation and infrastructure compatibility concerns.
Clients should accept the 2xx class rather than requiring exactly 200 and still
inspect GraphQL `errors`. Review status allowlists, caching rules, dashboards,
and retry policies when upgrading; 294 does not indicate that a mutation is safe
to retry. Match the numeric status rather than an HTTP reason phrase.

## Limitations and deployment boundaries

The oversized-URI difference is below the adapter boundary. In the locked
`actix-http 3.18.9` HTTP/1.1 path, URI conversion errors become generic 400
responses before routing. The corresponding Hyper path used by Axum distinguishes
URI overflow and returns 414. Correcting Actix's mapping requires changes below
the GraphQL handler or enforcement at an earlier server boundary. The existing
TCP regression covers this specific overflow case, not every request size or
protocol version.

Body limits use Axum's `DefaultBodyLimit` and Actix's `JsonConfig`; the
specification does not prescribe one universal byte limit. Application error
handlers can change responses. Header-size rejection (431), slow requests (408),
authentication failures, and proxy-generated errors must be verified for the
actual server configuration. The adapter suite is not evidence of every such
deployment behavior.

The runtime implements subscription execution and cancellation, but the adapters
do not provide SSE framing or the `graphql-transport-ws` protocol. These remain
separate work in [#32](https://github.com/Necrass-Dev/NecrassRs/issues/32) and
[#31](https://github.com/Necrass-Dev/NecrassRs/issues/31). Subscriptions are outside
the released HTTP specification's scope. Recognizing `text/event-stream` during
negotiation does not provide an SSE transport: valid SSE-only requests are
rejected with 406, and transport failures retain their status with an empty body.
See [Subscriptions](/docs/subscriptions/) for the implemented runtime behavior.

## Verification coverage

The [Axum adapter tests](https://github.com/Necrass-Dev/NecrassRs/blob/main/crates/necrassrs-axum/tests/http.rs)
and [Actix adapter tests](https://github.com/Necrass-Dev/NecrassRs/blob/main/crates/necrassrs-actix/tests/http.rs)
cover envelope validation, operation restrictions, content negotiation, request
errors, and execution results. Their 294 cases include queries and mutations,
partial data, root null, absent and weighted Accept headers, and preserved bodies.
The example servers add real HTTP/1.1 GET and POST checks, including root-null
execution errors, successful recovery, body limits, and the URI-overflow case.

Run these checks from the repository root:

```sh
cargo test -p necrassrs-http -p necrassrs-axum -p necrassrs-actix \
  -p axum-server@0.3.0 -p actix-server@0.3.0 --locked
```

Keep observed results and remaining conformance work in issues and pull requests.
Passing these checks does not establish full GraphQL or HTTP conformance.
