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

[GHSA-9gx9-5rq5-8523]: https://github.com/Necrass-Dev/NecrassRs/security/advisories/GHSA-9gx9-5rq5-8523
