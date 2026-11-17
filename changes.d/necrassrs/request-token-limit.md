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
