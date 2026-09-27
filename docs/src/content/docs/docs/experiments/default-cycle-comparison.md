---
title: "Input-default cycle validation: implementation comparison"
---

Date: 2026-09-27. Issue: [#23](https://github.com/Necrass-Dev/NecrassRs/issues/23).

## Decision and scope

Use field-default dependency traversal with completed-node reuse as the direction
for the Apollo schema-validation patch. Do not use uncached path expansion as the
production algorithm: even a small, valid schema can cause exponential repeated
work. The explicit graph prototype demonstrates the benefit, but its eager graph
construction and edge storage have measurable costs. These results do not select
an unmeasured lazy or compressed implementation.

Both candidates remain in a standalone [Rust example](https://github.com/Necrass-Dev/NecrassRs/blob/main/crates/necrassrs/examples/default_cycle_comparison.rs).
They are not wired into NecrassRs or Apollo validation. Existing #23 Red tests
remain failing. Variable-default coercion, dependency source overrides, Apollo
error diagnostics, and removal of wrapper workarounds are separate pending work.
No accepted representation or transport decision is changed. Working Draft and
transport revision candidates remain unconfirmed as documented in [specs.md](/docs/specs/).

## Algorithms

Both candidates use Apollo 1.33.0's parsed schema and AST. A node is an input-object
field with a declared default whose underlying named type is an input object.
Scalar/enum defaults cannot introduce input-object default cycles.

The shared literal walker follows explicitly supplied object/list values. For an
omitted field with an applicable default it calls an algorithm-specific callback.
Explicit null and empty lists terminate their paths. An explicit object does not
necessarily terminate expansion: its omitted fields can require defaults.

- **Path DFS:** the callback immediately expands the referenced default. An active
  field-coordinate set detects a repeated default on the current path; entries
  are removed on return. There is no completed-node cache across sibling paths or
  validation roots.
- **Explicit graph:** the callback records an edge without expanding its target.
  Each default literal is scanned once. A three-state DFS over the resulting
  adjacency lists detects cycles and reuses completed nodes. Repeated edge
  occurrences are retained; the prototype does not sort or deduplicate them.

Every input-field default can be reached by considering an empty object of its
own declaring input type, so checking all such defaults covers the default-cycle
rule. Dependencies of a given default are fixed by its declaration and explicit
literal: they do not depend on which caller requested the default. A default
expansion loop therefore corresponds to a cycle in this dependency graph.
This is not a graph of type references, and does not implement the separate
non-null type-cycle or draft unbreakable-cycle rules.

Reference: [September 2025 InputObjectDefaultValueHasCycle](https://spec.graphql.org/September2025/#sec-Input-Objects.Type-Validation).
The comparison assumes well-shaped input literals; general schema/default type
validation is not supplied by these experimental functions.

## Correctness checks

The executable and its test both run 29 expected-result checks (13 individual
cases and a 4-by-4 family). Expectations are asserted for each candidate, rather
than only asserting that the candidates agree.

The original #23 self-cycle, mutual cycle through lists, and terminating recursive
default structures are reused. Additional cases cover absent defaults, explicit
null, nested explicit termination, empty lists, null list items, singleton list
coercion, nested lists, finite sibling reuse, and dependencies that cross between
two fields. A non-null singular type cycle without defaults explicitly returns
false: it belongs to another validation rule, not this detector.

The exhaustive small family assigns absent, null, empty-object, or explicitly
terminating defaults independently to two recursive fields. It has a simple
independent expectation: a cycle exists exactly when either field defaults to
an empty object. Sharing the literal walker reduces implementation differences
but also means agreement alone cannot rule out a shared semantic bug. These
checks are not exhaustive conformance proof.

## Reproduction and measurement

From the repository root:

```sh
cargo test -p necrassrs --locked --example default_cycle_comparison
cargo run -p necrassrs --release --locked --example default_cycle_comparison
```

Recorded environment: aarch64-apple-darwin, Rust 1.96.1
(`31fca3adb283cc9dfd56b49cdee9a96eb9c96ffd`), Apollo Compiler 1.33.0,
Cargo release profile. No benchmark dependency was added.

Each case uses one warm-up followed by seven measured samples; the reported time
is the median. Parsing and the common list of default roots are outside the timed
region. Graph construction, algorithm-specific allocations, cycle traversal and
cleanup are inside it. Work counters are enabled for both candidates. Candidates
run in a fixed order, without process isolation or CPU pinning. Tiny timings are
especially noisy; counts and scaling are more useful evidence than speed ratios.
This is a schema-validation microbenchmark, not request throughput measurement.

[Raw results](/docs/experiments/default-cycle-results.csv) include SDL fragment bytes, default-node
count, median nanoseconds, field inspections, default expansions, stored edge
occurrences, and maximum active default depth. SDL byte counts exclude the common
query root appended by the harness. Depth does not count explicit literal nesting.

| Case                                        | Path median | Graph median, including construction | Path / graph default expansions | Stored graph edges |
| ------------------------------------------- | ----------: | -----------------------------------: | ------------------------------: | -----------------: |
| Self-cycle                                  |    0.375 us |                             0.459 us |                           1 / 1 |                  1 |
| Finite recursive default                    |    0.416 us |                             0.416 us |                           1 / 1 |                  0 |
| Branching depth 12                          |    3.095 ms |                             4.792 us |                     16,356 / 24 |                 44 |
| Branching depth 20                          |  438.094 ms |                             5.250 us |                  4,194,260 / 40 |                 76 |
| Chain depth 256                             |    3.387 ms |                            23.916 us |                    32,896 / 256 |                255 |
| Dense width 256                             |    5.843 ms |                             1.850 ms |                    66,048 / 512 |             65,536 |
| Immediate cycle followed by dense width 256 |    0.125 us |                             1.753 ms |                         1 / 513 |             65,537 |

The branching family has two defaults at each level referencing the next level.
Its finite expansion is exponential in depth, although validation need not build
that result. A simple chain exposes quadratic repeated suffix traversal when
uncached DFS starts from every default. The dense family has K root defaults
referencing a type with F defaulted fields, producing K*F edges. The final case
puts a self-cycle first and shows the cost of eager graph construction before
short-circuiting traversal.

## Complexity and memory interpretation

Let V be default nodes, E stored edge occurrences, P the total number of schema
fields inspected while scanning each explicit default literal once, and L the
size of explicit literals scanned. Assume expected constant-time hash operations
and bounded name lengths.

- Path DFS costs the work of all expanded paths: exponential for branching chains
  and quadratic for the all-roots linear-chain family. It retains active-path
  state rather than an adjacency graph.
- Graph construction costs expected O(L + P + E + V), including literal field
  lookup tables and the node index. Cycle traversal costs O(V + E). This is not
  necessarily linear in source size: E is quadratic for the dense family.
- Graph storage is O(V + E), plus traversal and temporary literal-walking state.
  Path DFS retains O(D) active coordinates and recursive frames, plus temporary
  lookup maps for explicit objects on the current path. The prototypes share
  parsed schema and root-list storage outside these algorithm-specific costs.

Memory was not measured with an allocator or RSS profiler. Structural counters
show the tradeoff: dense width 256 stores 65,536 usize edge entries, requiring
512 KiB of edge payload on this 64-bit target, before Vec capacities, headers,
node indices, DFS state and other allocations. Path DFS reaches active-default
depth 2 in that case. Neither figure is a total-memory measurement.

## Implications for the Apollo patch

1. Preserve field-default identity and explicit-value termination semantics.
   Type-only visited sets cannot distinguish legal recursion from default cycles.
2. Reuse completed dependency analysis to avoid repeatedly expanding shared
   defaults. Do not materialize coerced JSON just to validate a schema.
3. Keep the explicit graph implementation as the measured reference. If eager
   construction or edge storage is unacceptable, evaluate on-demand completed-node
   traversal against this same corpus before selecting it; do not claim results
   for an implementation that was not measured.
4. Integrate Apollo diagnostics and its traversal-depth safeguards before shipping.
   Both prototypes use native recursion and have no production recursion policy;
   passing the bounded corpus does not prove safety at arbitrary depth.
5. Keep public Apollo regression tests authoritative. This experiment does not
   make the existing Red tests pass and does not fix variable coercion.

Validation for this experiment: the example's expected-result test and release
run pass; workspace formatting and all-target Clippy pass. The workspace test
command reaches necrassrs with 50 passing tests and the two intentionally failing
#23 Red tests, then stops; later test targets were not run by that command.
