 -  Fixed `ExecutionOptions::argument_work_limit` counting a string argument
    as one value regardless of its length, so one large string variable
    passed under many aliases was copied for every alias without reaching the
    limit. Each string now also counts its length in bytes, and the default
    limit is raised from 1,000,000 to 4,000,000 so that a string argument as
    large as the default HTTP body limit still fits. Requests that pass many
    long strings to resolvers may now reach the limit. [GHSA-gc4x-f785-hfm2]

[GHSA-gc4x-f785-hfm2]: https://github.com/Necrass-Dev/NecrassRs/security/advisories/GHSA-gc4x-f785-hfm2
