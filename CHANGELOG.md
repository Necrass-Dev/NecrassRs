Changelog
=========

Version 0.3.0
-------------

Released on November 13, 2026.

### necrassrs

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

[#91]: https://github.com/Necrass-Dev/NecrassRs/issues/91

### necrassrs-cli

 -  Added prebuilt `necrass` CLI archives to GitHub Releases for Linux x86\_64,
    macOS Intel and Apple Silicon, and Windows x86\_64. Verify downloads using
    the SHA-256 digests provided by GitHub. [[#89]]

[#89]: https://github.com/Necrass-Dev/NecrassRs/issues/89


Version 0.2.1
-------------

Released on November 10, 2026.

### necrassrs

 -  Added `necrassrs::Request::operation_type(&self) -> Option<OperationType>`,
    which exposes the unambiguously selected operation type; schema validation
    still happens during execution.  [[#30]]
 -  Changed valid subscriptions sent to single-response execution to reject
    the request without creating a source. HTTP adapters return 406 for this
    rejection; execution errors continue to use 200.
    `necrassrs::Response::requires_response_stream(&self) -> bool` identifies
    this subscription rejection.  [[#30]]

### necrassrs-http

 -  Added
    `necrassrs_http::response_media_types<'a>(headers: impl IntoIterator<Item = &'a str>) -> Option<ResponseMediaTypes>`
    with `json: Option<&'static str>`, `legacy_json: bool`, and
    `event_stream: bool` fields, plus the `EVENT_STREAM` media-type constant.  [[#30]]

### necrassrs-axum

 -  Added support for GET queries, URL-encoded variables, and operation
    selection. Enable GET on the existing GraphQL handler alongside POST.
    Mutations over GET return 405 without executing resolvers. POST now
    requires an object envelope, validates `extensions`, and still ignores
    unknown properties. The `GraphQLRequest` extractor's
    `FromRequest::Rejection` changed from
    `axum::extract::rejection::JsonRejection` to `axum::response::Response`;
    update custom code that names or matches that associated type.  [[#30]]
 -  Fixed negotiated JSON media types on HTTP errors when clients accept both
    JSON response formats. GraphQL request errors for legacy JSON-only clients
    still use `application/graphql-response+json`. Method errors take priority
    over response negotiation; SSE-only transport errors keep their error
    status and send an empty body. Axum's native oversized-URI parser returns
    414 before application dispatch. This change does not implement SSE event
    delivery or WebSocket transport.  [[#30]]
 -  Moved GraphiQL to `/graphiql` in the maintained Axum and tutorial examples.
    `/graphql` now serves GET and POST GraphQL requests. Applications should
    expose `graphiql_html("/graphql")` on its own GET route and enable GET on
    their GraphQL handler. The UI remains opt-in, and its endpoint argument
    continues to identify the API route.  [[#30]]

### necrassrs-actix

 -  Added GET query support with URL-encoded variables and operation selection.
    Mount existing GraphQL handlers on both GET and POST to enable GET requests.
    GET mutations return 405 before resolver execution. POST requests now
    require an object envelope and validate the `extensions` field while
    continuing to ignore unknown properties.  [[#30]]
 -  Fixed HTTP error responses to preserve the negotiated JSON media type for
    clients accepting both JSON response formats. Legacy JSON-only clients
    continue to receive `application/graphql-response+json` for GraphQL request
    errors. Method failures take precedence over response negotiation. SSE-only
    transport failures retain their error status with an empty body. Native
    oversized-URI parser errors remain framework-owned: Actix returns 400
    before application dispatch. SSE event delivery and WebSocket transport
    are not implemented by this change.  [[#30]]
 -  Moved GraphiQL to `/graphiql` in the maintained Actix example, leaving
    `/graphql` for GET and POST GraphQL requests. Existing applications should
    register `graphiql_html("/graphql")` on a separate GET route and register
    their GraphQL handler for GET as well as POST. GraphiQL remains opt-in and
    its endpoint argument still points to the API route.  [[#30]]

### necrassrs-cli

 -  Moved GraphiQL to `/graphiql` in the CLI starters, leaving `/graphql` for
    GET and POST GraphQL requests. Open `/graphiql` to use the UI.  [[#30]]

[#30]: https://github.com/Necrass-Dev/NecrassRs/issues/30


Version 0.2.0
-------------

Released on November 5, 2026.

### necrassrs

 -  Changed NecrassRs releases to use one shared version tag and one set of
    product-level release notes for all eight public packages. This release
    changes release coordination only; applications upgrading from 0.1.2 do
    not require source changes.  [[#86]]

[#86]: https://github.com/Necrass-Dev/NecrassRs/issues/86


Version 0.1.2
-------------

Released on November 4, 2026.

### necrassrs

 -  Added GraphQL subscription execution, including source streams and response
    delivery.
 -  Fixed validation of merged subscription root selections and their response
    keys.

### necrassrs-build

 -  Added generated subscriber contracts and resolver source synchronization
    for GraphQL subscriptions.
 -  Made generated enum values cloneable so owned values can be delivered by
    subscription streams.


Version 0.1.1
-------------

Released on November 2, 2026.

### necrassrs

 -  Documented the public NecrassRs package boundaries and removed obsolete
    Apollo migration records.

### reed-compiler

 -  Documented the public Reed Compiler package boundary.

### reed-parser

 -  Documented the public Reed Parser package boundary.


Version 0.1.0
-------------

Initial release.  Released on November 2, 2026.

### necrassrs

 -  Added SDL-first GraphQL query and mutation execution with custom scalar
    support, composite results, input coercion, error propagation, and
    concurrent query fields.

### necrassrs-build

 -  Added generated Rust contracts and editable resolver synchronization.

### necrassrs-http

 -  Added shared GraphQL-over-HTTP negotiation for the Axum and Actix Web
    adapters.

### necrassrs-axum

 -  Added the Axum adapter with GraphQL-over-HTTP negotiation, request context
    isolation, introspection, and GraphiQL support.

### necrassrs-actix

 -  Added the Actix Web adapter with GraphQL-over-HTTP negotiation, request
    context isolation, introspection, and GraphiQL support.

### necrassrs-cli

 -  Added the `necrass` initializer with Axum and Actix Web project templates.

### reed-compiler

 -  Added the Reed GraphQL compiler package used by NecrassRs.

### reed-parser

 -  Added the Reed GraphQL parser package used by NecrassRs.
