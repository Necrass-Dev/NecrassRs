 -  Upgraded the `tracing` feature's OpenTelemetry dependencies from
    `opentelemetry` 0.31 and `tracing-opentelemetry` 0.32 to
    `opentelemetry` 0.33 and `tracing-opentelemetry` 0.34. This is a breaking
    change for applications that enable the `tracing` feature: their
    `opentelemetry`, `opentelemetry_sdk`, exporter, and
    `tracing-opentelemetry` dependencies must move to the same versions, or
    span names and OpenTelemetry context from NecrassRs will not reach their
    tracer provider. `opentelemetry_sdk` 0.33 also fixes GHSA-w9wp-h8wv-79jx,
    which affects 0.32.0 and earlier.
