 -  Fixed `introspection::check_max_depth` accepting operations that nest more
    introspection list fields than its limit when a named fragment is reused
    deeper than its first use or inside another fragment, including reuse
    primed by spreads skipped with `@skip`. Reused fragments now count toward
    the same limit as fragments traversed for the first time. Added
    `introspection::check_max_depth_with_limit` and
    `introspection::DEFAULT_MAX_LIST_DEPTH` so callers can choose how many
    nested `fields`, `inputFields`, `interfaces`, and `possibleTypes` lists
    an operation may select.
