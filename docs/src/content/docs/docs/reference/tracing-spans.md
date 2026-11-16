---
title: Tracing Spans
description: Span names, hierarchy, attributes, error classifications, and data-collection limits of NecrassRs tracing instrumentation.
---

These spans and events are emitted when the optional `tracing` Cargo feature is
enabled and an application subscriber collects them. See
[OpenTelemetry](/docs/opentelemetry/) to export them to a trace viewer.

This page describes the current source checkout. Published versions may not yet
include this feature.

## Span hierarchy

An exported query trace can look like this:

```text
HTTP request                         application middleware
└─ query                             SERVER; graphql.operation.name=GetOrders
   ├─ necrassrs.parse                INTERNAL
   ├─ necrassrs.validate             INTERNAL
   ├─ necrassrs.coerce_variables     INTERNAL
   └─ necrassrs.execute              INTERNAL
      └─ Query.orders               INTERNAL; path=orders
         └─ Order.customer          INTERNAL; path=orders.0.customer
            └─ database call        application instrumentation
```

The GraphQL operation's `SERVER` kind describes incoming GraphQL work even when
it is nested under an HTTP server span. It does not imply another HTTP request.

Operation names, kind, and standard attributes follow the
[GraphQL server span conventions](https://opentelemetry.io/docs/specs/semconv/graphql/graphql-spans/)
in OpenTelemetry Semantic Conventions **1.45.0**, reviewed for this documentation.
The GraphQL conventions have **Development** status. The `necrassrs.*` attributes,
phase and field spans, and error classifications below are NecrassRs contracts.

| Span                                | Scope                                                                       |
| ----------------------------------- | --------------------------------------------------------------------------- |
| `query`, `mutation`, `subscription` | Identified operation; includes request preparation                          |
| `GraphQL Operation`                 | Operation not identified, including parsing or operation-selection failure  |
| `necrassrs.parse`                   | Parsing the request document                                                |
| `necrassrs.validate`                | Building and validating the executable document and selecting the operation |
| `necrassrs.coerce_variables`        | Variable coercion, including custom scalar validation                       |
| `necrassrs.execute`                 | Query or mutation execution, including introspection processing             |

An identified operation records `graphql.operation.type` and, when named,
`graphql.operation.name`. Anonymous operations have no invented name attribute.
The operation name is not appended to the span name. Failed preparation does not
trust the client's operation-name string as a successfully selected operation.
Only phases that are reached produce spans. The document is not parsed again
solely for telemetry.

Query and mutation operation spans end when the GraphQL response is produced.
HTTP serialization and network transmission are outside that scope. Plain
`tracing` consumers see the static span names `GraphQL Operation` and
`GraphQL Field`; the bridge uses their `otel.name` fields for exported operation
and schema-coordinate names.

## Field spans

Field selection follows the policy of async-graphql 7.2.1:

| GraphQL return type                          | Default                    | `trace_scalars: true`      |
| -------------------------------------------- | -------------------------- | -------------------------- |
| Object, Interface, Union                     | Included                   | Included                   |
| Scalar, including custom scalars; Enum       | Omitted                    | Included                   |
| List or non-null wrapper                     | Use the wrapped named type | Use the wrapped named type |
| Introspection fields, including `__typename` | Omitted                    | Omitted                    |

Enable scalar and enum spans per execution using `execute_with_options`, or per
subscription using `subscribe_with_options`, passing these options as the final
argument:

```rust
let options = necrassrs::ExecutionOptions {
    trace_scalars: true,
    ..Default::default()
};
```

This setting requires the Cargo `tracing` feature to produce instrumentation.
There is no separate instrumentation depth limit. Merged selections produce
spans for actual field executions, and skipped fields produce none. Lists do not
create spans solely for their items; eligible fields within each item do.

Field spans have kind `INTERNAL` and use the concrete Object's schema coordinate,
such as `Order.customer`, even when selected through an Interface or Union.
`necrassrs.graphql.field.path` is a dot-separated response path with aliases and
list indices, such as `orders.0.buyer` for an aliased `customer` field.

A field's duration includes argument processing, its resolver, and result
completion, including descendant fields. It is not resolver-body-only time.
Nested durations overlap, and concurrent sibling durations can overlap; adding
all field durations does not give request wall-clock duration.

## Errors and cancellation

Operation status is determined from the GraphQL result, independently of HTTP
status. Partial data with GraphQL errors is a failed operation for tracing.

| Outcome                                                                         | OpenTelemetry status | `error.type`                          |
| ------------------------------------------------------------------------------- | -------------------- | ------------------------------------- |
| Response without errors, including a valid nullable `null`                      | `Unset`              | Absent                                |
| Parsing, validation, selection, input coercion, or subscription startup failure | `Error`              | `necrassrs.request_error`             |
| Partial data with errors, or `data: null` from non-null propagation             | `Error`              | `necrassrs.execution_error`           |
| Terminal subscription source failure                                            | `Error`              | `necrassrs.subscription_source_error` |

Errors generate events at their origin, without duplicating them during null
propagation. When a scalar or enum field span is omitted, its error event stays
on the nearest enclosing NecrassRs execution span. Field error events include
the response path and an `error.type` category: `necrassrs.argument_error`,
`necrassrs.resolver_error`, or `necrassrs.result_error`. Multiple execution errors
retain the common operation-level `necrassrs.execution_error` classification.

Dropping unfinished execution or a subscription records
`necrassrs.cancelled=true` on the operation and closes its spans. Cancellation
alone does not set `Error`; errors already observed are not erased. This does not
roll back resolver side effects or stop independently spawned application tasks.

The [recommended bridge configuration](/docs/opentelemetry/#3-replace-the-stdout-setup) disables automatic status changes from error
events with `with_error_events_to_status(false)`. A recovered application error
log should not mark a successful GraphQL response as failed. Different subscriber
layers or bridge settings can change the final telemetry interpretation.

## Subscription spans

The `subscription` operation span begins at startup, including synchronous request
preparation, and stays open until exhaustion, terminal failure, or cancellation.
Each response computation has a `necrassrs.subscription.event` span with kind
`INTERNAL`. Waiting for source events contributes to the overall subscription
duration, but not to event-processing duration.

Event fields follow the same field-selection options and response-path rules.
A response with GraphQL errors marks that event and the overall subscription as
failed. Later events can still succeed, with their own status left unset. A
terminal source error closes the stream; startup failure closes the operation
without starting event processing.

The overall span may not be exported until a long-lived subscription ends.
Completed event spans provide visibility while it is active. Keep telemetry
providers alive until streams have ended or been dropped. See
[Subscriptions](/docs/subscriptions/) for stream ownership and cancellation.

## Data collection and limits

Framework instrumentation does not automatically record raw query documents
(`graphql.document`), variables, argument values, response data, raw error
messages, error extensions, authentication headers, or application Context
values. Operation names and response paths are recorded, so client-supplied names
and aliases remain visible. Application logs and HTTP or database instrumentation
have their own data-collection policies.

Operation information is discovered after parsing and validation. Head sampling
based on the operation name at span creation is therefore not guaranteed. Use
parent-based or probability-based sampling without adding an extra parse.

Large lists can generate many field spans, especially with `trace_scalars`
enabled. Review the SDK's
[span limits](https://opentelemetry.io/docs/specs/otel/trace/sdk/#span-limits),
exporter queue capacity, and your collection system's trace-size and retention
limits. Attribute or event limits apply per span; they do not cap the number of
spans in a trace. Long-lived subscriptions may also exceed a backend's trace
assembly window. NecrassRs does not impose a separate span-count or depth limit.
