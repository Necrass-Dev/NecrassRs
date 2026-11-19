 -  Fixed executable validation overflowing the thread stack when fragment
    spreads compose a deeply nested selection. Validation now rejects any
    operation or fragment, including unused fragments, whose selection depth
    exceeds 500, reporting a single `too much recursion` error and skipping
    the rest of validation. A definition's own selection set has depth 1, and
    each nested selection set entered through a field, an inline fragment, or
    a fragment spread adds one level. Such documents no longer report other
    diagnostics alongside the recursion error.
