Changelog
=========

Version 0.4.8
-------------

Released on November 28, 2026.

### necrassrs

 -  Fixed `ExecutionOptions::argument_work_limit` counting a string argument
    as one value regardless of its length, so one large string variable
    passed under many aliases was copied for every alias without reaching the
    limit. Each string now also counts its length in bytes, and the default
    limit is raised from 1,000,000 to 4,000,000 so that a string argument as
    large as the default HTTP body limit still fits. Requests that pass many
    long strings to resolvers may now reach the limit. [GHSA-gc4x-f785-hfm2]
 -  Fixed a small request making execution do unbounded work. Fragment
    spreads that double the selected fields at every level, aliases repeating a
    long returned list, aliased `__typename` under list items, and aliases of
    `__schema` could each make a request of a few kilobytes execute millions
    of fields or use more than a gigabyte of memory. The new
    `ExecutionOptions::field_limit` and `ExecutionOptions::list_item_limit`
    bound the fields executed and the list items completed in one response,
    each with a default of 50,000; work is charged before it starts, and a
    response over either limit drops resolvers that are still pending and is
    replaced by `data: null` and a single error.
    Top-level mutation fields are charged together, so a mutation over the
    field limit performs no root field. The new
    `ExecutionOptions::introspection_node_limit` bounds the values schema
    introspection generates, with a default of 100,000, and an operation over
    it fails as a request error. Each subscription event has its own budget.
    GraphQL requests cannot override these server settings. Responses that
    legitimately exceed a default, such as 5,000 list items with ten fields
    each, now fail until the application raises the limit. Existing
    exhaustive `ExecutionOptions` struct literals must add these fields or
    use `..Default::default()`. [GHSA-gc4x-f785-hfm2]

[GHSA-gc4x-f785-hfm2]: https://github.com/Necrass-Dev/NecrassRs/security/advisories/GHSA-gc4x-f785-hfm2

### necrassrs-cli

 -  The starter's `src/options.rs` now lists every `ExecutionOptions` field
    with its default value instead of `..Default::default()`, so each
    execution limit can be adjusted in place. Upgrading NecrassRs with a new
    option makes the file fail to compile until the new field is added.

### reed-compiler

 -  Added `Execution::response_node_limit`, which bounds the values completed
    for one response, and `introspection::partial_execute_with_node_limit`.
    Each field value and list item counts once, and execution stops as soon as
    the limit is exceeded with a request error instead of a partial response.
    Both `Execution` and `introspection::partial_execute` now apply
    `resolvers::DEFAULT_RESPONSE_NODE_LIMIT`, 100,000 values, by default;
    previously the response size was unbounded, so aliases of `__schema`
    could generate gigabytes of introspection data. Callers that need larger
    responses must set a higher limit. [GHSA-gc4x-f785-hfm2]


Version 0.4.7
-------------

Released on November 25, 2026.

### necrassrs

 -  Fixed request validation and field argument coercion work growing
    quadratically with the number of fields that pass operation variables as
    arguments. Responses and errors are unchanged. [GHSA-hq2r-87c2-972j]
 -  Fixed resolver argument work multiplying with aliases and returned list
    items. A large variable or list literal used by many aliases, or by a
    field selected under every item of a long returned list, was copied for
    every resolver call, so a small request could keep the server busy for
    seconds. Fields under a returned list now share one coerced argument map,
    and the new `ExecutionOptions::argument_work_limit` bounds the argument
    values passed to resolvers in one response, with a default of 1,000,000.
    A field over the limit fails with a field error and null propagates as for
    other field errors; each subscription event has its own budget. GraphQL
    requests cannot override this server setting. Existing exhaustive
    `ExecutionOptions` struct literals must add this field or use
    `..Default::default()`. [GHSA-9gx9-5rq5-8523]

[GHSA-hq2r-87c2-972j]: https://github.com/Necrass-Dev/NecrassRs/security/advisories/GHSA-hq2r-87c2-972j
[GHSA-9gx9-5rq5-8523]: https://github.com/Necrass-Dev/NecrassRs/security/advisories/GHSA-9gx9-5rq5-8523

### reed-compiler

 -  Fixed executable-document validation work growing quadratically with the
    number of variable references, such as many aliased fields, repeated
    arguments, or list items that use operation variables. Validation results
    and diagnostics are unchanged. [GHSA-hq2r-87c2-972j]


Version 0.4.6
-------------

Released on November 22, 2026.

### necrassrs

 -  Upgraded the `tracing` feature's OpenTelemetry dependencies from
    `opentelemetry` 0.31 and `tracing-opentelemetry` 0.32 to
    `opentelemetry` 0.33 and `tracing-opentelemetry` 0.34. This is a breaking
    change for applications that enable the `tracing` feature: their
    `opentelemetry`, `opentelemetry_sdk`, exporter, and
    `tracing-opentelemetry` dependencies must move to the same versions, or
    span names and OpenTelemetry context from NecrassRs will not reach their
    tracer provider. `opentelemetry_sdk` 0.33 also fixes GHSA-w9wp-h8wv-79jx,
    which affects 0.32.0 and earlier.

### necrassrs-actix

 -  Changed the `actix-web` dependency to require 4.19 or later and to disable
    its default features. `necrassrs-actix` no longer enables Actix Web's
    HTTP/2, compression, cookie, WebSocket, or macro features on the
    application's behalf, so applications that disable them no longer pull in
    `h2` 0.3, `brotli`, or `zstd`. An application that uses one of these
    features must enable it on its own `actix-web` dependency; applications
    that declare `actix-web = "4"` keep the defaults. Run
    `cargo update -p actix-web -p actix-http` to pick up the fixed Actix Web
    and Actix HTTP releases for RUSTSEC-2026-0446, RUSTSEC-2026-0447,
    RUSTSEC-2026-0448, RUSTSEC-2026-0455, RUSTSEC-2026-0552,
    RUSTSEC-2026-0553, and RUSTSEC-2026-0554; `actix-http` 3.21.3 or later
    includes every fix. [[#101]]

[#101]: https://github.com/Necrass-Dev/NecrassRs/issues/101

### necrassrs-cli

 -  Changed the Actix Web project created by `necrass init --framework actix`
    to depend on `actix-web` 4.19 or later with default features disabled and
    only the `macros` feature enabled. New projects no longer resolve Actix Web
    and Actix HTTP releases affected by recent advisories, and no longer pull in
    `h2` 0.3, which is affected by RUSTSEC-2026-0258, or `brotli` and `zstd`.
    Enable features such as `http2` or the `compress-*` features on the
    project's `actix-web` dependency when the application needs them.
    [[#101]]


Version 0.4.5
-------------

Released on November 22, 2026.

### necrassrs

 -  Fixed introspection requests bypassing the introspection list depth limit
    by reusing a named fragment deeper than its first use, which let a small
    request produce exponentially large introspection responses. Such requests
    now fail as request errors before introspection runs, and the maintained
    adapters return 422. Added
    `ExecutionOptions::introspection_list_depth_limit` so applications can
    change the limit; the default of 2 keeps the previous limit. GraphQL
    requests cannot override this server setting. Existing exhaustive
    `ExecutionOptions` struct literals must add this field or use
    `..Default::default()`.

### reed-compiler

 -  Fixed `introspection::check_max_depth` accepting operations that nest more
    introspection list fields than its limit when a named fragment is reused
    deeper than its first use or inside another fragment, including reuse
    primed by spreads skipped with `@skip`. Reused fragments now count toward
    the same limit as fragments traversed for the first time. Added
    `introspection::check_max_depth_with_limit` and
    `introspection::DEFAULT_MAX_LIST_DEPTH` so callers can choose how many
    nested `fields`, `inputFields`, `interfaces`, and `possibleTypes` lists
    an operation may select.


Version 0.4.4
-------------

Released on November 19, 2026.

### necrassrs

 -  Fixed a request within the default token limit aborting the server
    process by overflowing the stack of a Tokio worker thread during
    validation, even with introspection disabled. Requests whose composed
    selection depth exceeds 500 now fail as validation errors with a single
    `too much recursion` error, and the maintained adapters return 422.

### reed-compiler

 -  Fixed executable validation overflowing the thread stack when fragment
    spreads compose a deeply nested selection. Validation now rejects any
    operation or fragment, including unused fragments, whose selection depth
    exceeds 500, reporting a single `too much recursion` error and skipping
    the rest of validation. A definition's own selection set has depth 1, and
    each nested selection set entered through a field, an inline fragment, or
    a fragment spread adds one level. Such documents no longer report other
    diagnostics alongside the recursion error.


Version 0.4.3
-------------

Released on November 17, 2026.

### necrassrs

 -  Added `ExecutionOptions::token_limit` so applications can change the
    request parser token limit. GraphQL requests cannot override this server
    setting. Existing exhaustive `ExecutionOptions` struct literals must add
    this field or use `..Default::default()`.
 -  Fixed malformed GraphQL documents producing one syntax error per invalid
    character, which let a small request allocate and return a very large
    error response. Request parsing now stops after 15,000 lexical tokens by
    default, matching Apollo Router's `parser_max_tokens`. Whitespace, commas,
    comments, and invalid characters count as tokens, and documents over the
    limit are rejected as syntax errors.


Version 0.4.2
-------------

Released on November 16, 2026.

### necrassrs

 -  Added `ExecutionOptions::validation_work_multiplier` so applications can
    choose the relative executable-validation limit. The default is 8; a value
    of zero applies only the fixed 2,048-work minimum. GraphQL requests cannot
    override this server setting. Existing exhaustive `ExecutionOptions` struct
    literals must add this field or use `..Default::default()`.

### necrassrs-cli

 -  Added editable `src/options.rs` to new Axum and Actix projects. Both
    generated handlers use its `ExecutionOptions` for request validation and
    execution. Existing projects can adopt the same `execute_with_options` call.

### reed-compiler

 -  Fixed excessive executable-document validation work when multiple operations
    reuse shared fragments. Documents that exceed the validation work limit now
    return a validation error; applications may need to reduce repeated work in
    such documents. Applications can set the relative work multiplier through
    `ValidationOptions`; its default remains 8.


Version 0.4.1
-------------

Released on November 15, 2026.

### reed-parser

 -  Fixed excessive CPU use when parsing GraphQL string literals with malformed
    braced Unicode escapes. Valid Unicode escapes retain their existing
    behavior. [GHSA-frqr-7pr4-8x92]

[GHSA-frqr-7pr4-8x92]: https://github.com/Necrass-Dev/NecrassRs/security/advisories/GHSA-frqr-7pr4-8x92


Version 0.4.0
-------------

Released on November 13, 2026.

### necrassrs

 -  Added `Response::is_execution_error` to distinguish completed execution
    results containing errors from request errors. [[#93]]

[#93]: https://github.com/Necrass-Dev/NecrassRs/issues/93

### necrassrs-axum

 -  Changed completed execution results containing errors to return HTTP `294`,
    while preserving the negotiated JSON response body. [[#93]]

### necrassrs-actix

 -  Changed completed execution results containing errors to return HTTP `294`,
    while preserving the negotiated JSON response body. [[#93]]


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
