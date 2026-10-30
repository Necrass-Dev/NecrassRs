---
title: "Local Apollo correction and upstream work"
---

NecrassRs maintains its own Apollo Compiler patch for
[#23](https://github.com/Necrass-Dev/NecrassRs/issues/23) and OneOf support
and JSON numeric variable coercion in [#24](https://github.com/Necrass-Dev/NecrassRs/issues/24).
The compiler and parser patches also carry language, validation, and introspection
corrections for [#28](https://github.com/Necrass-Dev/NecrassRs/issues/28). The separate
[dodok8/apollo-rs fork](https://github.com/dodok8/apollo-rs) is a preparation area
for the minimal upstream implementation and regression tests. It is not a local
path dependency of NecrassRs. The two repositories have different change scopes.

## NecrassRs-owned artifacts

- [Compiler source patch](../patches/apollo-compiler-1.33.0.patch), relative to
  the compiler crate root, records changes to 20 library source files.
- [Parser source patch](../patches/apollo-parser-0.8.6.patch), relative to the
  parser crate root, records changes to 10 library source files.
- `vendor/apollo-compiler` and `vendor/apollo-parser` contain the corresponding
  crates.io library sources with those patches applied, READMEs, license texts,
  and `.cargo_vcs_info.json`. The compiler also retains the example embedded by
  its rustdoc. External upstream test fixtures, benches, and other examples are
  omitted; in-source tests remain in the source copies.
- The build-only manifests omit upstream development targets/dependencies. The
  compiler's direct parser path selects the maintained parser from the same
  checkout. These are packaging adaptations, excluded from the source patches.
- Direct dependency regressions, subprocess checks, executor cases, external
  consumer tests, and Cargo/CLI wiring remain NecrassRs integration work.

| Package | Upstream commit and path | crates.io archive SHA-256 |
| --- | --- | --- |
| Apollo Compiler 1.33.0 | [`5ebcc40bdf6919843c122e9a76b241689edcd6ef`](https://github.com/apollographql/apollo-rs/tree/5ebcc40bdf6919843c122e9a76b241689edcd6ef/crates/apollo-compiler), `crates/apollo-compiler` | `5cf9cb85c9c600cc8ff8154d502a4c8bbf9abb8c3f915d34914c1950a1a6273a` |
| Apollo Parser 0.8.6 | [`e36ca6dceb668b4b0c703449beea39e75c16ead5`](https://github.com/apollographql/apollo-rs/tree/e36ca6dceb668b4b0c703449beea39e75c16ead5/crates/apollo-parser), `crates/apollo-parser` | `11f6e2be4b3474e5890d34d3e483d49ec9b5cbf9e358bc62c9cc5423376df54b` |

The archive hashes were verified locally. Both upstream license texts are resolved
from each package's base commit, replacing the release archives' relative-path
placeholders; `.cargo_vcs_info.json` retains the release provenance. Parser license
sources: [Apache-2.0](https://github.com/apollographql/apollo-rs/blob/e36ca6dceb668b4b0c703449beea39e75c16ead5/LICENSE-APACHE)
and [MIT](https://github.com/apollographql/apollo-rs/blob/e36ca6dceb668b4b0c703449beea39e75c16ead5/LICENSE-MIT).

To reproduce the source changes, extract each unmodified release archive and run
its command from that extracted crate's root:

```sh
# From the apollo-compiler-1.33.0 root:
patch -p1 < /path/to/apollo-compiler-1.33.0.patch
# From the apollo-parser-0.8.6 root:
patch -p1 < /path/to/apollo-parser-0.8.6.patch
```

On 2026-10-30, both patches applied cleanly to fresh release extractions, and every
resulting `src/` file matched the corresponding vendored file byte for byte.

Keep both vendored source trees and their patches synchronized. Each source diff excludes
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

## Language, validation, and introspection corrections

The #28 patches accept descriptions on executable definitions, accept explicit
empty selection sets while distinguishing them from absent selections, and support
applied directives on directive definitions and directive extensions. Unicode
string decoding accepts valid surrogate pairs and variable-width escapes; invalid
scalar values remain lexical errors. Lexer acceptance and CST decoding share the
same escape decoder.

The compiler validates required-input deprecation restrictions, compatible defaults,
and input-object field uniqueness, including nested custom scalar literals.
`InputObjectHasUnbreakableCycle` follows every ancestor in a path: an ordinary
input must satisfy all required singular fields, while a OneOf input needs at least
one finite alternative. Scalar, enum, list, and optional ordinary-input edges allow
termination. This check is separate from the #23 default-expansion dependency graph.

Introspection reports `isOneOf`, directive deprecation metadata and filtering,
`DIRECTIVE_DEFINITION`, and the pinned non-null `includeDeprecated` arguments.
The complete regression inventory and generated-consumer evidence are maintained
in [specs.md](specs.md#issue-28-regression-inventory).

## Cargo and consumers

The NecrassRs root selects both maintained source copies:

```toml
[patch.crates-io]
apollo-compiler = { path = "vendor/apollo-compiler" }
apollo-parser = { path = "vendor/apollo-parser" }
```

No sibling Apollo checkout is required. The vendored compiler directly depends on
`../apollo-parser`, so a consumer selecting that compiler also gets the patched
parser from the same NecrassRs checkout. External consumer roots must declare their own override because Cargo
does not inherit one from a dependency manifest:

```toml
[patch.crates-io]
apollo-compiler = { path = "../NecrassRs/vendor/apollo-compiler" }
```

Adjust the path for the checkout. Current CLI templates use the repository default
branch for framework dependencies and the compiler override. The first consumer
build records resolved revisions in its lockfile. The compiler override is:

```toml
[patch.crates-io]
apollo-compiler = { git = "https://github.com/Necrass-Dev/NecrassRs.git", version = "1.33.0" }
```

Local automated CLI tests substitute the maintained local source. A separate
remote-consumer check on 2026-09-27 used the generated manifest unchanged for its
initial build, fetched the earlier Git revision `cc3a3b2a77f42b91d3fe417c48e346682f26dc7c`,
and compiled successfully. This is historical evidence, not verification of the
current #28 changes. Cargo metadata
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
repeating the check. Repeat these checks after updating the consumer lockfile;
a successful historical remote build does not validate an unpublished local branch.

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
cargo test -p necrassrs --locked --test apollo_conformance --test validation_clauses
cargo test -p necrassrs --locked cyclic_
cargo test -p necrassrs-build --test cargo_build --locked
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The workspace tests compile the maintained dependencies but do not run their full
upstream development suites. Those suites were not run for this checkpoint. Run
them from an appropriately patched full upstream checkout, not the build-only copy:

```sh
cargo test -p apollo-compiler --test main input_defaults
cargo test -p apollo-compiler
```

Remove the local patch/source override only after an upstream release passes the
portable regressions and external consumer checks. Update the root lockfile and
all root/template overrides together. The #23 pinned specification comparison is recorded in [specs.md](specs.md).
The pinned core specification difference inventory is recorded there as well.
Subscription and transport work remains separate; local patch verification is not
full parent-issue conformance.
