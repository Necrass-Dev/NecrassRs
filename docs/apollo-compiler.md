# Local Apollo correction and upstream work

NecrassRs maintains its own Apollo Compiler patch for
[#23](https://github.com/Necrass-Dev/NecrassRs/issues/23). The separate
[dodok8/apollo-rs fork](https://github.com/dodok8/apollo-rs) is a preparation area
for the minimal upstream implementation and regression tests. It is not a local
path dependency of NecrassRs. The two repositories have different change scopes.

## NecrassRs-owned artifacts

- [Source patch](../patches/apollo-compiler-1.33.0.patch), relative to the compiler
  crate root, records the five validation/coercion source changes.
- `vendor/apollo-compiler` is a minimal buildable copy of the crates.io 1.33.0
  package with that patch applied. It contains library source, licenses, provenance,
  README, and the single example embedded by the library's rustdoc. Upstream test
  fixtures, benches, other examples, and development dependencies are omitted.
  The build-only manifest is a packaging adaptation, not an upstream change.
- [Algorithm experiments](experiments/default-cycle-comparison.md) and their raw
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
existing coercion. The first cycle diagnostic includes available field locations.
Type recursion and the draft unbreakable-cycle rule remain separate checks.

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

Adjust the path for the checkout. The CLI starter's intended Git override points
to the NecrassRs repository, where this maintained source lives, not the upstream
PR branch. Local tests substitute the local source. Remote use requires this work
to be published first; no published revision containing the patch is claimed.
After publication, pin an immutable revision and verify the consumer lockfile.

## Upstream PR scope

The separate fork has local branch `fix/input-defaults`, based on
`e106f94195762e9ace7b09655a9be0dc6d4d3d76`. Its production patch contains compiler implementation, diagnostics, and native
regression tests. It also keeps a portable comparison example, the historical
benchmark CSV, and an experiment record as supporting evidence for discussion.
Those artifacts distinguish prototype measurements from the production patch;
whether to include them in the upstream PR remains open. It does not carry
NecrassRs manifests or source packaging. Its diff is independently reviewable and
may evolve differently during upstream review.

No upstream issue/PR or push has been made for this correction. Publication is
pending maintainer discussion. Upstream's existing compatibility discussion is
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
all root/template overrides together. Parent specification-revision candidates
remain unconfirmed; local patch verification is not full conformance.
