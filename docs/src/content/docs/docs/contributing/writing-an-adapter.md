---
title: Writing an Adapter
description: Connect another HTTP framework to the NecrassRs execution core using the public runtime API.
---

An adapter can live in your application or in a separate crate. There is no adapter registration API or required adapter trait in the execution core. Implement your framework's native extraction and response conversion interfaces, or perform the conversion in a handler. The `GraphQLRequest` and `GraphQLResponse` names are conventions used by the maintained adapters.

See [Integration](/docs/integration/) for how adapters fit between the framework and the execution core, and the [GraphQL over HTTP reference](/docs/graphql-over-http/) for the contract an adapter implements.

Use these implementations as references:

- [Axum adapter source](https://github.com/Necrass-Dev/NecrassRs/blob/main/crates/necrassrs-axum/src/lib.rs): `FromRequest`, `IntoResponse`, and negotiation middleware.
- [Actix Web adapter source](https://github.com/Necrass-Dev/NecrassRs/blob/main/crates/necrassrs-actix/src/lib.rs): `FromRequest`, `Responder`, and negotiation through the request object.
- [Shared HTTP source](https://github.com/Necrass-Dev/NecrassRs/blob/main/crates/necrassrs-http/src/lib.rs): framework-independent response media-type selection.

## Extract the request

Use your framework's JSON parser and body-limit configuration. A POST body must be an object containing a required string `query`, optional object `variables` and `extensions`, and an optional string `operationName`; ignore unknown properties. The maintained adapters accept `null` for optional fields and treat it as absent. For GET, URL-decode `query` and `operationName`, parse `variables` and `extensions` as JSON objects, and normalize empty optional parameters as absent. Reject malformed JSON, missing or incorrectly typed fields, unsupported content types, and oversized bodies using the [status contract](/docs/graphql-over-http/#status-and-error-contract).

Construct `Request::new(query)`, then call `with_variables(variables)` and `with_operation_name(operation_name)` when those values are present. Use `necrassrs::JsonMap` for variables. Request construction does not validate GraphQL; pass the request to execution and let the runtime handle document validation and coercion.

## Negotiate the response type

Depend directly on `necrassrs-http` to reuse `response_media_types`. Pass all `Accept` header values as strings, preserving multiple values. An empty iterator means the header is absent and selects legacy `application/json`. Reject malformed values and requests that accept neither JSON nor SSE before GraphQL execution. `response_media_type` remains available when only ordinary JSON negotiation is needed.

Negotiation needs access to the incoming headers. If your framework's response conversion interface cannot access the request, select the type in route middleware or carry it from extraction to response conversion. Apply negotiation to the GraphQL execution route, preserving framework method rejections and separate HTML routes.

## Convert the runtime response

Let the application handler call `execute` or `execute_with_options`; it owns the schema, dispatcher, and Context. Convert the returned `Response` using its public classification methods:

1. `requires_response_stream()` selects 406 for ordinary JSON delivery.
2. Otherwise, `is_syntax_error()` selects 400.
3. Otherwise, `is_request_error()` selects 422.
4. Otherwise, `is_execution_error()` selects 294, including partial data and `data: null` with errors.
5. Error-free execution selects 200.

Serialize `Response` directly with Serde. Use the negotiated media type, switching a non-2xx response to `necrassrs_http::GRAPHQL_JSON` only when `ResponseMediaTypes::legacy_json` is true. Include `Vary: Accept` without discarding existing values. Keep framework extraction errors separate from GraphQL response serialization.

Use the runtime classification methods rather than matching diagnostic text, counting serialized errors, or checking whether `data` is null. Those approaches lose the distinction between request and execution errors. Leave resolver errors, field paths, and null propagation to the runtime.

## Verify and document the integration

Exercise the adapter through your framework's HTTP test utilities. Cover request fields and types, variables and operation selection, per-request Context, the status table, and negotiation with absent, repeated, weighted, excluded, and invalid `Accept` values. Check that unacceptable response types prevent execution, body-limit configuration still works, and unrelated routes retain their own behavior.

The [Axum HTTP tests](https://github.com/Necrass-Dev/NecrassRs/blob/main/crates/necrassrs-axum/tests/http.rs) and [Actix Web HTTP tests](https://github.com/Necrass-Dev/NecrassRs/blob/main/crates/necrassrs-actix/tests/http.rs) provide concrete cases. Include a runnable consumer showing handler registration, application state, and Context construction, and document any transport limitations or differences from this contract.

Use the [GraphQL over HTTP verification guidance](/docs/graphql-over-http/#verification-coverage)
for adapter and real HTTP consumer checks. The maintained behavior follows the
September 2026 edition; the [documented limitations](/docs/graphql-over-http/#limitations-and-deployment-boundaries)
remain part of the support contract. Neither these tests nor an external checker
establish complete HTTP conformance.

## Add a maintained adapter

A reusable adapter does not automatically become a CLI option. To add a maintained framework to this repository, add its adapter and tests to the Cargo workspace, supply a consumer example, and update the [supported frameworks table](/docs/integration/#supported-frameworks). If the CLI should generate projects for it, also add its framework choice, starter files, and generated-project checks in `necrassrs-cli`.
