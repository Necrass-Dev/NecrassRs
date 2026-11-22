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
