 -  Added `Execution::response_node_limit`, which bounds the values completed
    for one response, and `introspection::partial_execute_with_node_limit`.
    Each field value and list item counts once, and execution stops as soon as
    the limit is exceeded with a request error instead of a partial response.
    Both `Execution` and `introspection::partial_execute` now apply
    `resolvers::DEFAULT_RESPONSE_NODE_LIMIT`, 100,000 values, by default;
    previously the response size was unbounded, so aliases of `__schema`
    could generate gigabytes of introspection data. Callers that need larger
    responses must set a higher limit. [GHSA-gc4x-f785-hfm2]

[GHSA-gc4x-f785-hfm2]: https://github.com/Necrass-Dev/NecrassRs/security/advisories/GHSA-gc4x-f785-hfm2
