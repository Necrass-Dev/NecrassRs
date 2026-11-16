---
title: Subscriptions
description: Declare subscription fields, implement event sources, consume response streams, and handle event errors and cancellation.
---

A subscription creates a source stream once, then executes the selected GraphQL
fields for each event. Each event produces its own response. The application
supplies the event source and owns the request Context.

NecrassRs supports subscription execution and generated resolver contracts.
WebSocket and Server-Sent Events (SSE) transports are not included in the current
HTTP adapters. See [Integration](/docs/integration/#subscription-transports) for
the transport boundary, and [Subscription Console](/docs/subscription-console/)
for a complete runnable example.

These descriptions follow the current source checkout; published package versions
may differ.

## Select one root field

A schema with a Subscription root still requires a Query root. Each subscription
operation selects one root response key. Repeated selections with the same response key can merge, but
two different aliases count as two keys. Root introspection fields and root
`@skip` or `@include` selections are rejected, including through fragments.
Nested fields can use conditional directives normally.

## Implement the event source

Subscription root fields implement `Subscriber::resolve`. Query, Mutation, and
Object fields implement `Resolver::resolve`. Generated marker and argument paths,
naming, and source ownership rules are the same as for other fields; see
[Organizing Resolver Files](/docs/resolver-files/).

`Subscriber::Output` is one value of the SDL field, not a stream type. Nullable,
list, scalar, enum, and abstract results use the same
[type mappings](/docs/types/) as other resolver outputs. Selected Object fields
run against each event's own value.

`SourceStream::from_stream` accepts a `Send` stream of `SourceEvent<Output>`.
The input stream does not need to be `Sync`, `Unpin`, or `'static`; it may borrow
the subscriber and Context for `'a`. The subscriber is `Send + Sync`, Context
is `Sync`, and the source-creation future and event values are `Send`.
Borrowing a source does not enable borrowed Object outputs, which remain unsupported.

Construct and authorize Context before starting the subscription; the runtime
does not create or refresh it between events.

## Start the subscription

Use `subscribe` for subscriptions and `execute` for queries and mutations.
`subscribe_with_options` accepts `ExecutionOptions` as its final argument.
Subscription root introspection remains invalid regardless of those options.

The dispatcher constructor takes Query, Mutation, then Subscription roots,
omitting roots absent from the SDL.

Startup validates the document and coerces variables once, then creates the
selected source once. `subscribe` returns `Ok(ResponseStream)` after creation
finishes, without consuming the first event. The original `Request` can then be
dropped. Keep the schema, dispatcher, and Context alive while consuming
the response stream. The startup future is `Send`; `ResponseStream` is `Send`
and `Unpin` and yields `Result<Response, SubscriptionError>`.

## Handle errors and termination

| Situation                                                       | Result                                                         | Later events                  |
| --------------------------------------------------------------- | -------------------------------------------------------------- | ----------------------------- |
| Invalid request or source creation returns `Err(ResolverError)` | `subscribe` returns `Err(Response)` with errors and no `data`  | No response stream is created |
| Source yields `SourceEvent::Value(value)`                       | `Ok(Response)` after completing selected fields                | Continue                      |
| Source yields `SourceEvent::FieldError(error)`                  | `Ok(Response)` with a root-field execution error               | Continue                      |
| A selected field fails or a result cannot be serialized         | `Ok(Response)` with GraphQL errors and normal null propagation | Continue                      |
| Source yields `SourceEvent::SourceError(error)`                 | `Err(SubscriptionError::Source(error))`                        | Terminate                     |
| Source is exhausted                                             | `None`                                                         | Finished                      |

For example, use `FieldError` when one event cannot produce the root field's
value but the source can still deliver later events. Use `SourceError` when the
source has failed permanently. Errors returned while creating the source are
startup errors, including invalid custom scalar arguments.

An event response with `data: null` does **not** end the subscription. When the
root field is non-null, a root `FieldError` makes the event's data null. The next
successful event can still produce an ordinary response.
Execution errors retain field locations, alias-aware paths, and application
extensions.

`SubscriptionError::Internal` represents an exceptional runtime termination;
it is not an ordinary GraphQL field error. The API does not promise to catch
arbitrary application panics. A terminal stream error is yielded once, resources
are released, and later polls return `None`. Normal exhaustion also releases the
source without requiring an additional poll.

## Ordering, cancellation, and resource ownership

The optional `tracing` feature records the overall subscription lifetime and
each event's response computation. See [Tracing Spans](/docs/tracing-spans/#subscription-spans)
for field selection, error status, and cancellation attributes.

- Responses within one subscription preserve source-event order.
- Independent selected fields and list items within an event can make concurrent
  progress. Response ordering remains stable.
- Different subscriptions can make independent progress when polled concurrently.
- Dropping the response stream cancels its source and any in-progress field
  futures. Dropping startup before it finishes drops the source-creation future.
  Resources owned by those futures and streams are released through Rust's drop
  behavior.

The current executor completes each event before requesting the next and uses
no detached tasks or unbounded event queue for this flow. These are implementation
choices; future scheduling changes must account for resolver side effects,
buffering, ordering, and cancellation.

Your application owns any external producers and buffers: choose their
capacity, overflow policy, and cleanup behavior. Dropping a subscription does not
automatically stop a separately spawned producer. Reconnection, replay,
acknowledgements, and durable delivery also belong to the application and transport.
