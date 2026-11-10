Changelog
=========

Unreleased
----------

To be released.

 -  Added GET query support to the Axum and Actix adapters, including
    URL-encoded variables and operation selection. GET mutations return 405
    before resolver execution. POST requests now require an object envelope and
    validate the `extensions` field while continuing to ignore unknown
    properties. Mount existing GraphQL handlers on both GET and POST to enable
    GET requests.
    `necrassrs::Request::operation_type(&self) -> Option<OperationType>`
    exposes the unambiguously selected operation type; schema validation still
    happens during execution. The Axum `GraphQLRequest` extractor's
    `FromRequest::Rejection` changed from
    `axum::extract::rejection::JsonRejection` to `axum::response::Response`;
    update custom code that names or matches that associated type.  [[#30]]
 -  Fixed HTTP error responses to preserve the negotiated JSON media type for
    clients accepting both JSON response formats. Legacy JSON-only clients
    continue to receive `application/graphql-response+json` for GraphQL request
    errors. Valid subscriptions sent to single-response execution now return
    HTTP 406 without creating a source; execution errors continue to use 200.
    `necrassrs::Response::requires_response_stream(&self) -> bool` identifies
    this subscription rejection. Added
    `necrassrs_http::response_media_types<'a>(headers: impl IntoIterator<Item = &'a str>) -> Option<ResponseMediaTypes>`
    with `json: Option<&'static str>`, `legacy_json: bool`, and
    `event_stream: bool` fields, plus the `EVENT_STREAM` media-type constant.
    SSE-only transport failures retain their error status with an empty body;
    SSE event delivery and WebSocket transport are not implemented by this
    change.  [[#30]]
 -  Moved GraphiQL to `/graphiql` in the CLI starters and maintained examples,
    leaving `/graphql` for GET and POST GraphQL requests. Open `/graphiql` to
    use the UI. Existing applications should register
    `graphiql_html("/graphql")` on a separate GET route and register their
    GraphQL handler for GET as well as POST. GraphiQL remains opt-in and its
    endpoint argument still points to the API route.  [[#30]]

[#30]: https://github.com/Necrass-Dev/NecrassRs/issues/30


Version 0.2.0
-------------

Released on November 5, 2026.

 -  Changed NecrassRs releases to use one shared version tag and one set of
    product-level release notes for all eight public packages. This release
    changes release coordination only; applications upgrading from 0.1.2 do
    not require source changes.  [[#86]]

[#86]: https://github.com/Necrass-Dev/NecrassRs/issues/86


Version 0.1.2
-------------

Released on November 4, 2026.

 -  Added GraphQL subscription execution, including generated subscriber
    contracts, source streams, response delivery, and resolver source
    synchronization.
 -  Made generated enum values cloneable so owned values can be delivered by
    subscription streams.
 -  Fixed validation of merged subscription root selections and their response
    keys.


Version 0.1.1
-------------

Released on November 2, 2026.

 -  Documented the public Reed Parser, Reed Compiler, and NecrassRs package
    boundaries and removed obsolete Apollo migration records.


Version 0.1.0
-------------

Initial release.  Released on November 2, 2026.

 -  Added the Reed GraphQL parser and compiler packages used by NecrassRs.
 -  Added SDL-first GraphQL query and mutation execution with generated Rust
    contracts, editable resolver synchronization, custom scalar support,
    composite results, input coercion, error propagation, and concurrent query
    fields.
 -  Added Axum and Actix Web adapters with GraphQL-over-HTTP negotiation,
    request context isolation, introspection, and GraphiQL support.
 -  Added the `necrass` initializer with Axum and Actix Web project templates.
