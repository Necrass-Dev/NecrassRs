 -  Added optional OpenTelemetry integration through the `tracing` Cargo feature
    and an application-configured `tracing-opentelemetry` subscriber layer.
    [[#91]]
     -  Added GraphQL operation spans with operation type and name, plus
        separate parsing, validation, variable-coercion, and execution spans to
        locate request bottlenecks. Custom scalar validation and introspection
        are included in their respective phases.
     -  Added Object, Interface, and Union field spans by default, with
        concrete schema coordinates and dot-separated response paths containing
        aliases and list indices. Set `ExecutionOptions::trace_scalars` to
        `true` to include scalar and enum fields; introspection fields remain
        excluded. Field duration includes arguments, the resolver, and
        descendant completion.
     -  Added operation error status for request failures, partial GraphQL
        errors, and null propagation, origin error events without propagation
        duplicates, and `necrassrs.cancelled=true` for cancelled operations.
        Framework telemetry excludes raw documents, input and response values,
        error messages and extensions, authentication headers, and application
        Context values.
     -  Added subscription lifetime and per-event processing spans, including
        event errors, terminal source errors, and cleanup on cancellation.
        Completed event spans can be exported before the overall subscription
        ends.
     -  Added an OpenTelemetry manual and Axum and Actix Web examples with HTTP
        parent spans, parent-based sampling, stdout export, and provider
        shutdown. Applications retain ownership of telemetry configuration and
        distributed context propagation.
 -  Changed `ExecutionOptions` to include `trace_scalars: bool`, defaulting to
    `false`. This breaks exhaustive struct literals even without the `tracing`
    feature. Replace `ExecutionOptions { introspection: false }` with
    `ExecutionOptions { introspection: false, ..Default::default() }`, or supply
    `trace_scalars` explicitly. This change follows the minor-version policy for
    0.x releases. [[#91]]
