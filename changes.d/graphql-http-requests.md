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
