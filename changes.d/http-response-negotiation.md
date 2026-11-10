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
