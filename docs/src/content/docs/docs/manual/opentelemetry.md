---
title: OpenTelemetry
description: Observe GraphQL request phases, field execution, errors, and subscriptions using tracing and OpenTelemetry.
---

NecrassRs instrumentation is built on Rust's `tracing` crate. Enable the optional
`tracing` Cargo feature to emit structured spans and error events, then attach a
`tracing-opentelemetry` subscriber layer to export them through OpenTelemetry.
A span represents a timed unit of work; parent and child spans connect a GraphQL
operation to its phases, fields, and application calls. An event records a point
within that work, such as an error originating at a field.

This lets you distinguish request preparation from field execution and locate
slow or failing fields by their response paths. The runtime instruments calls
into Reed at the NecrassRs boundary; Reed's parser and validation internals do
not emit individual spans.

This page describes the current source checkout. Published versions may not yet
include this feature.

## Send your first trace to Jaeger

OpenTelemetry is the instrumentation and export system; it does not itself
provide a trace-viewing screen. This walkthrough sends traces to **Jaeger**, which
receives them and provides a browser UI. It uses the repository's Axum example so
you can start with an existing schema and resolver instead of building a server
from scratch.

You need a checkout containing the `tracing` feature, a working Rust toolchain,
and Docker or Podman running. Run the commands from the repository root.
The application runs on your host, and Jaeger runs in a container.

```text
GraphQL request → NecrassRs tracing spans → tracing-opentelemetry layer
                → OpenTelemetry SDK → OTLP/HTTP exporter → Jaeger → browser UI
```

### 1. Start the trace receiver

In one terminal, run:

```sh
docker run --rm --name necrassrs-jaeger \
  -p 127.0.0.1:16686:16686 \
  -p 127.0.0.1:4318:4318 \
  cr.jaegertracing.io/jaegertracing/jaeger:2.23.0
```

With Podman, use `podman run` with the same arguments instead of `docker run`.
On macOS, Podman also needs a running Linux virtual machine. Use
`podman machine list` to check its state, `podman machine init` if none exists,
and `podman machine start` to start it before running the container.

Leave it running. Open [http://localhost:16686](http://localhost:16686) to confirm
that the UI is available. There will be no application traces yet.

Port **16686** is the UI; **4318** receives OTLP over HTTP. The exporter will send
to `http://127.0.0.1:4318/v1/traces`, not the UI address. This local setup uses
Jaeger's in-memory storage: stopping the container loses its traces. See
[Jaeger's getting-started guide](https://www.jaegertracing.io/docs/2.23/getting-started/)
for the all-in-one configuration.

### 2. Add the exporter dependency

The unmodified `examples/axum-server` writes traces to stdout. It already enables
`necrassrs`'s `tracing` feature and includes the OpenTelemetry SDK and subscriber
layer. A stdout exporter does **not** send anything to Jaeger.

In `examples/axum-server/Cargo.toml`, add this entry to `[dependencies]`:

```toml
opentelemetry-otlp = { version = "0.31", default-features = false, features = ["trace", "http-proto", "reqwest-blocking-client"] }
```

Also add `signal` to the existing Tokio feature list for graceful Ctrl-C shutdown:

```toml
tokio = { version = "1.53.1", features = ["macros", "net", "rt-multi-thread", "signal"] }
```

Keep the example's other dependencies. These versions match its OpenTelemetry
`0.31` / `tracing-opentelemetry` `0.32` combination. The
[OTLP exporter documentation](https://docs.rs/opentelemetry-otlp/0.31.0/opentelemetry_otlp/)
describes its HTTP and gRPC transports; this walkthrough uses HTTP with protobuf.

### 3. Replace the stdout setup

In `examples/axum-server/src/main.rs`, replace the existing telemetry imports and
`init_tracing` function with the following. Keep the remaining imports, schema,
handler, and `app()` function.

```rust
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::{Resource, trace::{Sampler, SdkTracerProvider}};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

fn init_tracing() -> SdkTracerProvider {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint("http://127.0.0.1:4318/v1/traces")
        .build()
        .expect("create the OTLP exporter");

    let provider = SdkTracerProvider::builder()
        .with_resource(Resource::builder().with_service_name("necrassrs-example").build())
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)))
        .with_batch_exporter(exporter)
        .build();

    tracing_subscriber::registry()
        .with(
            tracing_opentelemetry::layer()
                .with_error_events_to_status(false)
                .with_tracer(provider.tracer("graphql-server")),
        )
        .try_init()
        .expect("initialize the application subscriber only once");
    provider
}
```

`necrassrs-example` is the **service name you will select in Jaeger**.
`graphql-server` is the tracer's instrumentation-scope name, not the service name.
The sampler records all new root traces for this local exercise and follows a
parent's decision when one exists. The batch exporter sends completed spans in
batches, so a trace may take a few seconds to appear.

The `with_error_events_to_status(false)` setting lets NecrassRs set status from
the GraphQL result instead of treating every application error log as an
operation failure.

### 4. Initialize before serving and flush on shutdown

Replace the example's `main` function with:

```rust
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let provider = tokio::task::spawn_blocking(init_tracing)
        .await
        .map_err(std::io::Error::other)?;

    let server = async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
        axum::serve(listener, app())
            .with_graceful_shutdown(async {
                tokio::signal::ctrl_c().await.expect("listen for Ctrl-C");
            })
            .await
    }
    .await;

    let shutdown = tokio::task::spawn_blocking(move || provider.shutdown())
        .await
        .map_err(std::io::Error::other)?
        .map_err(std::io::Error::other);
    server?;
    shutdown
}
```

The provider stays alive for the server's lifetime. Ctrl-C stops accepting
requests, waits for graceful shutdown, and then exports remaining completed
spans. Initialization and shutdown of this blocking HTTP exporter run on Tokio's
blocking thread pool. Abruptly killing the process can still lose buffered spans.

The existing `TraceLayer::new_for_http()` in `app()` supplies an HTTP parent span.
The existing `graphql` handler already calls `necrassrs::execute`; no resolver
changes are needed to produce GraphQL spans.

### 5. Run the application and send a request

In a second terminal, run the modified example:

```sh
cargo run -p axum-server
```

In a third terminal, send a request using the example's existing schema:

```sh
curl -sS http://127.0.0.1:3000/graphql \
  -H 'Content-Type: application/json' \
  --data '{"query":"query Greeting { hello(name: \"Sheri\") }"}'
```

The expected response is:

```json
{ "data": { "hello": "Hello, Sheri" } }
```

### 6. Find the trace

In [Jaeger](http://localhost:16686), refresh the page after a few seconds. Select
**necrassrs-example** under **Service**, leave **Span Name** at **All Span Names**,
and click **Find Traces**. Open the result named **necrassrs-example: request**;
`request` is the HTTP parent. Select its child `query` span. Its details should show
`graphql.operation.name=Greeting`, with parsing, validation, variable-coercion,
and execution spans beneath it.

You will not see `Query.hello` with default options because it returns a scalar.
To inspect that field too, replace the final `GraphQLResponse(...)` expression
in the example's `graphql` handler with:

```rust
GraphQLResponse(
    necrassrs::execute_with_options(
        &state.schema,
        &request,
        &state.dispatcher,
        &context,
        necrassrs::ExecutionOptions {
            trace_scalars: true,
            ..Default::default()
        },
    )
    .await,
)
```

Restart the application and resend the request. The new trace should include
`Query.hello` with `necrassrs.graphql.field.path=hello`. See
[Select field spans](#select-field-spans) for the default policy.

When finished, press Ctrl-C in the application terminal, then in the Jaeger
terminal. The container is removed automatically by `--rm`.

### If no trace appears

| Symptom                                    | Check                                                                                                                                       |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Nothing under the service selector         | Send a request, wait for batch export, refresh, and widen the search time range. Confirm that `init_tracing` runs before the server starts. |
| Traces only appear in terminal output      | Replace the stdout exporter with the OTLP exporter in step 3. Starting Jaeger alone does not change the application's exporter.             |
| Export fails or connection is refused      | Keep Jaeger running and use `http://127.0.0.1:4318/v1/traces`. Port 16686 is the UI; 4317 is OTLP/gRPC, not this example's HTTP transport.  |
| Only some requests appear                  | The original repository examples sample about 10% of root traces. Use the local sampler shown above while learning.                         |
| Operation spans appear, but no field spans | Scalar and enum fields are omitted by default. Use `trace_scalars: true` for the `hello` example.                                           |
| Subscriber initialization fails            | Install one application subscriber and add the OpenTelemetry layer to it; do not initialize a second global subscriber.                     |

## Apply the setup to your application

Enable `features = ["tracing"]` on your existing `necrassrs` dependency, keeping
its version, Git revision, or path. This feature is available starting with
NecrassRs **0.3.0**. In a workspace that declares the dependency, a member uses:

```toml
[dependencies]
necrassrs = { workspace = true, features = ["tracing"] }
```

Add the SDK, bridge, subscriber, and exporter to your application dependencies:

```toml
[dependencies]
opentelemetry = { version = "0.31", default-features = false, features = ["trace"] }
opentelemetry_sdk = { version = "0.31", default-features = false, features = ["trace"] }
opentelemetry-otlp = { version = "0.31", default-features = false, features = ["trace", "http-proto", "reqwest-blocking-client"] }
tracing-opentelemetry = { version = "0.32", default-features = false }
tracing-subscriber = { version = "0.3.23", default-features = false, features = ["registry"] }
```

Reuse the initialization and shutdown pattern above with your service name and
receiver's OTLP endpoint. If the receiver requires TLS or authentication,
configure the exporter's TLS features and credentials according to that service.
If the application also runs in a container, `127.0.0.1` refers to that container;
use a receiver address reachable from the application's network.

The runtime's feature is disabled by default. It enables `tracing`, the
OpenTelemetry API, and `tracing-opentelemetry`, but does not install an SDK or a
subscriber. Without a subscriber that collects the instrumentation, enabling the
feature alone produces no exported traces. Other `tracing` layers can also
consume the spans.

Applications own sampling, exporters, HTTP context propagation, and shutdown.
For production traffic, choose a sampling ratio and exporter capacity appropriate
to your workload rather than retaining the local all-root-traces sampler.
This setup exports traces and events, not an OpenTelemetry logs exporter or a
metrics pipeline. A separate OpenTelemetry Collector is optional for this local
path because Jaeger accepts OTLP directly; a deployed system can send to a
Collector or another OTLP-compatible service instead.

### HTTP parents and application calls

The [Axum example](https://github.com/Necrass-Dev/NecrassRs/tree/main/examples/axum-server)
uses `tower_http::trace::TraceLayer`; the
[Actix Web example](https://github.com/Necrass-Dev/NecrassRs/tree/main/examples/actix-server)
uses `tracing_actix_web::TracingLogger`. Both install the bridge and shut down
the provider after the server future returns. Run either from the repository root:

```sh
cargo run -p axum-server
# Or, in a separate process:
cargo run --manifest-path examples/actix-server/Cargo.toml
```

Axum listens on port 3000 and Actix Web on 3001. Send GraphQL requests as described
in [Integration](/docs/integration/). The GraphQL operation inherits the active
HTTP span; HTTP span names and kinds are controlled by the middleware.

An HTTP parent span by itself does not establish distributed context propagation.
Configure your HTTP instrumentation to extract incoming `traceparent` and
`tracestate` and attach the extracted OpenTelemetry context to the HTTP span.
Configure outgoing clients to inject context as needed. The examples demonstrate
local HTTP parentage, not an explicit incoming-header extraction setup.

Database and outbound HTTP instrumentation also belong to the application. When
these calls run under an active GraphQL field span, their spans can appear as its
children. For asynchronous application work, use `tracing::Instrument` rather
than holding a span's entered guard across an `.await`.

## Read the span hierarchy

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

## Select field spans

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

## Interpret errors and cancellation

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

The bridge configuration above disables automatic status changes from error
events with `with_error_events_to_status(false)`. A recovered application error
log should not mark a successful GraphQL response as failed. Different subscriber
layers or bridge settings can change the final telemetry interpretation.

## Observe subscriptions

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

## Upgrade existing execution options

Starting with NecrassRs **0.3.0**, `ExecutionOptions` includes
`trace_scalars: bool`, defaulting to `false`. When upgrading from an earlier
version, an exhaustive literal such as `ExecutionOptions { introspection: false }`
must include the new field or use defaults:

```rust
let options = necrassrs::ExecutionOptions {
    introspection: false,
    ..Default::default()
};
```

This source compatibility change also applies when the `tracing` Cargo feature
is disabled. It is a minor-version change under the repository's
[0.x version policy](/docs/contributing/#changelogs-and-releases).
