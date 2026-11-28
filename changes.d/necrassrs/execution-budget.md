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
