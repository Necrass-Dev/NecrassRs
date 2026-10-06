---
title: "Local Apollo correction and upstream work"
---

NecrassRs maintains its own Apollo Compiler patch for
[#23](https://github.com/Necrass-Dev/NecrassRs/issues/23) and OneOf support
and JSON numeric variable coercion in [#24](https://github.com/Necrass-Dev/NecrassRs/issues/24). The separate
[dodok8/apollo-rs fork](https://github.com/dodok8/apollo-rs) is a preparation area
for the minimal upstream implementation and regression tests. It is not a local
path dependency of NecrassRs. The two repositories have different change scopes.

## NecrassRs-owned artifacts

- [Source patch](https://github.com/Necrass-Dev/NecrassRs/blob/main/patches/apollo-compiler-1.33.0.patch), relative to the compiler
  crate root, records the eight library source changes.
- `vendor/apollo-compiler` is a minimal buildable copy of the crates.io 1.33.0
  package with that patch applied. It contains library source, licenses, provenance,
  README, and the single example embedded by the library's rustdoc. Upstream test
  fixtures, benches, other examples, and development dependencies are omitted.
  The build-only manifest is a packaging adaptation, not an upstream change.
- [Algorithm experiments](default-cycle-comparison.md) and their raw
  results remain in NecrassRs as the implementation decision record.
- Direct dependency regressions, subprocess checks, executor cases, external
  consumer tests, and Cargo/CLI wiring remain NecrassRs integration work.

The package base is upstream commit
`5ebcc40bdf6919843c122e9a76b241689edcd6ef`, at `crates/apollo-compiler`.
The release archive SHA-256 was verified as
`5cf9cb85c9c600cc8ff8154d502a4c8bbf9abb8c3f915d34914c1950a1a6273a`.
Both upstream license texts (resolved from the base commit, replacing the release
package's relative-path placeholders) and `.cargo_vcs_info.json` are retained.

To reproduce the source changes, extract that release and apply the patch from
the extracted compiler crate root:

```sh
patch -p1 < /path/to/apollo-compiler-1.33.0.patch
```

Keep the vendored source and this patch synchronized. The source diff excludes
NecrassRs packaging changes and can be reviewed independently. The corrected
schema validator builds a field-default dependency graph and traverses it
iteratively. Variable-default and input-field-default branches reuse Apollo's
existing coercion while preserving validated scalar literal representations.
Supplied JSON values retain scalar validation; recursive list/object processing
keeps the distinction between supplied values and validated defaults. The first cycle diagnostic includes available field locations.
Type recursion and the draft unbreakable-cycle rule remain separate checks.

Supplied JSON numbers with an empty fractional part are integer inputs. The
patch normalizes in-range Int variables to integer JSON values before generated
conversion, including nested lists and input fields. ID variables accept signed
and unsigned integer values and normalize integer-valued floating representations
to strings. Fractional values remain invalid for Int and ID. GraphQL literal
validation is unchanged: floating-point literals remain invalid at Int and ID
positions. `json_variable_numbers_are_normalized_without_changing_literal_kinds`
checks this directly through Apollo, and a compiled generated consumer checks
the same behavior through resolver dispatch and result serialization.

Validated ID integer literals are converted directly from their AST text to
strings before JSON number parsing. This preserves integers beyond signed,
unsigned, or floating-point ranges in variable and nested input-field defaults,
including singleton and nested lists. The NecrassRs argument coercion path uses
the same representation for request literals and argument defaults. Public Apollo
and compiled-consumer regressions cover positive/negative large IDs and 400-digit
integers without rounding.

The patch also rejects non-null OneOf input fields and any OneOf field default,
including explicit null, with source locations for the invalid type or default.
`@oneOf` is registered as a non-repeatable built-in directive without arguments,
restricted to input object definitions. Existing explicit declarations remain
supported. Document validation requires exactly one supplied field with a
non-null literal or a compatible non-null member variable declaration, even if
a nullable variable has a non-null default. It checks nested objects and list
elements before variable coercion, so a second field using an undefined variable
cannot disappear before the cardinality check. Existing unknown-field validation
continues to apply.

Variable-value coercion checks OneOf selection cardinality and non-nullness
before recursively coercing the selected field, then checks that its coerced
value remains non-null. The same path handles nested objects, list elements,
and variable defaults. Nullable whole objects and nullable list items retain
their ordinary semantics. Invalid supplied selections produce request errors
before execution; `crates/necrassrs/tests/apollo_one_of.rs` exercises these rules
directly through Apollo's public API.

## Cargo and consumers

The NecrassRs root selects its own source copy:

```toml
[patch.crates-io]
apollo-compiler = { path = "vendor/apollo-compiler" }
```

No sibling Apollo checkout is required to build NecrassRs. `apollo-parser` remains
a registry dependency; a fork's workspace configuration does not leak into this
lockfile. External consumer roots must declare their own override because Cargo
does not inherit one from a dependency manifest:

```toml
[patch.crates-io]
apollo-compiler = { path = "../NecrassRs/vendor/apollo-compiler" }
```

Adjust the path for the checkout. The CLI starter pins all three NecrassRs packages and the Apollo override to
`ffd953c8c56496677f62583f96794396b5f126c9`, containing the numeric default regression fix.
This revision must be published before remote consumers can fetch it. The Apollo override is:

```toml
[patch.crates-io]
apollo-compiler = { git = "https://github.com/Necrass-Dev/NecrassRs.git", rev = "ffd953c8c56496677f62583f96794396b5f126c9", version = "1.33.0" }
```

Local automated CLI tests substitute the maintained local source. A separate
remote-consumer check on 2026-09-27 used the generated manifest unchanged for its
initial build, fetched the earlier Git revision `cc3a3b2a77f42b91d3fe417c48e346682f26dc7c`,
and compiled successfully. This is historical evidence, not verification of the
new numeric-fix pin. Cargo metadata
showed exactly one Apollo Compiler package at that Git source/revision, shared by
`necrassrs` and `necrassrs-build`; no sibling checkout or local path override was
used.

The generated consumer then passed the four portable `apollo_defaults` tests
(copied into its `tests/` directory with `apollo-compiler = "1.33.0"` as a dev
dependency) and a generated-dispatch execution test. The latter rejected a cyclic
schema and returned `Hello, Sheri` from the generated greeting resolver. These
five tests passed using the remote dependencies. The test-only additions also
included `serde_json = "1.0"`; production dependencies were unchanged.

Reproduce the remote build from this repository:

```sh
cargo run -p necrassrs-cli --locked -- init /tmp/apollo-patch-consumer
cargo build --manifest-path /tmp/apollo-patch-consumer/Cargo.toml
cargo metadata --manifest-path /tmp/apollo-patch-consumer/Cargo.toml --locked --format-version 1
```

Use a fresh destination. Inspect `source` for Apollo and dependency edges from
both runtime and build packages. Keep the generated consumer's lockfile when
repeating the check. A future source update must update the pins and repeat these
checks; the selected commit is not a moving branch.

### Numeric default regression follow-up

The regression `numeric_literal_defaults_preserve_validated_values` covers Float
`9007199254740991` and ID `9223372036854775808` as variable and input-field
defaults, with scalar, singleton-list, and nested-list types (18 combinations).
It also rejects invalid explicitly supplied values on each path. The TDD Red
checkpoint is commit `57308d8`; the local fix preserves validated scalar literals
instead of applying JSON variable numeric restrictions to them.

At the numeric-fix checkpoint, CLI templates pinned the corrected source. The
recorded remote checks above remain historical evidence for those pinned revisions.

Current CLI templates use the repository default branch for NecrassRs dependencies
and the Apollo patch. The generated consumer's first build records the resolved
revisions in its lockfile. Local CLI checks validate both framework templates
against the maintained checkout; they do not establish the behavior of a future
remote revision. See [getting started](https://necrass.rs/docs/installation/#create-a-project) for the current
installation and dependency policy.

## Upstream PR scope

The separate fork has local branch `fix/input-defaults`, based on
`e106f94195762e9ace7b09655a9be0dc6d4d3d76`. Its production patch contains compiler implementation, diagnostics, and native
regression tests. It also keeps a portable comparison example, the historical
benchmark CSV, and an experiment record as supporting evidence for discussion.
Those artifacts distinguish prototype measurements from the production patch;
whether to include them in the upstream PR remains open. It does not carry
NecrassRs manifests or source packaging. Its diff is independently reviewable and
may evolve differently during upstream review.

The maintainer handles the Apollo fork separately. This record makes no claim
about its current publication or review status; NecrassRs acceptance does not
require an upstream merge. Upstream's existing compatibility discussion is
[#928](https://github.com/apollographql/apollo-rs/issues/928); stricter default-cycle
validation rejects schemas previously accepted by 1.33.0.

## Verification and removal

From NecrassRs:

```sh
cargo test -p necrassrs --locked --test apollo_defaults
cargo test -p necrassrs --locked cyclic_
cargo test -p necrassrs-build --test cargo_build --locked
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Run upstream's own tests from the separate fork, not the build-only source copy:

```sh
cargo test -p apollo-compiler --test main input_defaults
cargo test -p apollo-compiler
```

Remove the local patch/source override only after an upstream release passes the
portable regressions and external consumer checks. Update the root lockfile and
all root/template overrides together. The #23 pinned specification comparison is recorded in [specs.md](specs.md).
Transport reference candidates and the complete parent difference inventory remain
separate work; local patch verification is not full conformance.
