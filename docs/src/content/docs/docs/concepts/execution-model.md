---
title: Execution Model
description: How NecrassRs prepares a request, schedules resolvers, reports errors, propagates null, and handles cancellation.
---

NecrassRs executes GraphQL requests against SDL-generated resolver contracts.
The application owns the validated schema, the generated dispatcher, and the
Context value; the runtime borrows them for each request. HTTP adapters only
convert requests and responses around this model; see [Integration](/docs/integration/).

This page describes the current source checkout. Use
[docs.rs](https://docs.rs/necrassrs/) for the API of a published version.

## Request lifecycle

`execute` and `execute_with_options` run a query or mutation in two stages:

1. **Preparation.** Parse the document, validate it against the schema, select
   the operation, and coerce variables.
2. **Execution.** Resolve the selected fields through the dispatcher, then
   complete each result according to its GraphQL type.

The schema is parsed once at startup with `Schema::parse_and_validate`, usually
from `generated::SDL`, and reused across requests. Each request supplies its own
Context, which the runtime passes to resolvers without creating, cloning, or
refreshing it.

Subscriptions use `subscribe` instead. Preparation and source creation happen
once, then execution runs for each source event; see
[Subscriptions](/docs/subscriptions/).

## Request errors and execution errors

A failure during preparation produces a **request error**: the response contains
`errors` and no `data` entry, and no resolver runs. A failure during execution
produces an **execution error**: the response contains `data`, possibly `null`,
together with `errors` that carry the field's response path and locations.

| Response classification      | Meaning                                                |
| ---------------------------- | ------------------------------------------------------ |
| `is_syntax_error()`          | The document could not be parsed                       |
| `is_request_error()`         | Preparation failed; `data` is absent                   |
| `requires_response_stream()` | A subscription was passed to single-response execution |
| `is_execution_error()`       | Execution produced one or more field errors            |

Resolvers return `ResolverError` for application failures. Its message and
optional extensions become an execution error at that field. Argument coercion
and result conversion failures are execution errors at the same position.
Transports should use these classifications rather than inspecting error text
or whether `data` is null; [GraphQL over HTTP](/docs/graphql-over-http/#status-and-error-contract)
maps them to HTTP statuses.

## Null propagation

When a field fails, its value becomes `null`. If the field's type is non-null,
the `null` propagates to the nearest nullable ancestor field or list item, and
that ancestor becomes `null` instead. If no nullable ancestor exists, the
response's `data` is `null`. The error is reported once, at the field where it
originated.

SDL nullability therefore decides how much of a response survives a failure.
A nullable field isolates its own errors; a non-null field trades that isolation
for a stricter type.

## Scheduling and ordering

| Selection                       | Scheduling                               |
| ------------------------------- | ---------------------------------------- |
| Query root fields               | Polled concurrently                      |
| Fields in nested selection sets | Polled concurrently                      |
| List items                      | Polled concurrently                      |
| Top-level mutation fields       | Awaited one at a time, in document order |

Selections below each mutation field use normal concurrent execution. A mutation
field that propagates `null` to the operation root prevents later top-level
mutation fields from starting.

Concurrency does not change response ordering: Object fields keep query order,
list items keep input order, and errors are merged in the corresponding order.
Applications must not rely on the start or completion order of sibling
resolvers. Execution does not spawn independent tasks or impose a concurrency
limit.

## Cancellation

The future returned by `execute` borrows the schema, dispatcher, request, and
Context, and is `Send`. Dropping it drops in-progress field futures. This does
not roll back external effects that resolvers have already performed, and it
does not stop tasks the application spawned independently.

## Panics and unimplemented resolvers

The runtime does not convert panics into GraphQL errors. Newly generated
resolver stubs call `unimplemented!()`, and applications configure
`panic = "abort"` so that calling one terminates the process instead of
returning a partial response. See
[Build Configuration](/docs/build-configuration/#cargo-setup) for the profile
setup. Return `ResolverError` for every failure that should reach the client.

## Execution options

`ExecutionOptions` holds server-controlled settings passed to
`execute_with_options` or `subscribe_with_options`:

| Field                            | Default | Effect                                                                                                                                                                                                           |
| -------------------------------- | ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `introspection`                  | `true`  | When `false`, selected `__schema` and `__type` fields are rejected before resolver calls; `__typename` remains available                                                                                         |
| `introspection_list_depth_limit` | `2`     | Rejects introspection operations that nest more introspection list fields as request errors; see [GraphQL over HTTP](/docs/graphql-over-http/#limitations-and-deployment-boundaries)                             |
| `trace_scalars`                  | `false` | Adds tracing spans for scalar and enum fields; see [Tracing Spans](/docs/tracing-spans/#field-spans)                                                                                                             |
| `validation_work_multiplier`     | `8`     | Rejects documents whose validation work exceeds this multiple of their size, with a 2,048 minimum, as validation errors; see [GraphQL over HTTP](/docs/graphql-over-http/#limitations-and-deployment-boundaries) |
| `token_limit`                    | `15000` | Rejects request documents with more lexical tokens as syntax errors; see [GraphQL over HTTP](/docs/graphql-over-http/#limitations-and-deployment-boundaries)                                                     |

Construct options with `..Default::default()` so that new fields keep their
defaults. See [GraphiQL](/docs/graphiql/) for introspection policy.
