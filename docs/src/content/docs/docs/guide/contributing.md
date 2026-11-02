---
title: "Contributing"
description: Develop, validate, package, and release changes to NecrassRs.
---

NecrassRs welcomes bug reports, design discussions, documentation improvements, and code contributions. Before changing the repository, read the relevant issue, [architecture](https://github.com/Necrass-Dev/NecrassRs/blob/main/archives/architecture.md), and [AI usage policy](https://github.com/Necrass-Dev/NecrassRs/blob/main/AI_POLICY.md). Use an issue's specified branch when it has one, and keep unrelated changes in separate commits.

## Validate changes

Start with the smallest check that exercises the changed behavior. Rust changes must pass the workspace baseline before review:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Run the documentation checks after changing this website:

```sh
cd docs
pnpm install --frozen-lockfile
pnpm lint
pnpm fmt:check
pnpm check
pnpm build
```

Bug fixes and nontrivial behavior changes need a runnable regression check. Generator changes must also compile a generated consumer. Report checks that could not run and do not replace a failing assertion with an incorrect expectation.

## Public package versions

The repository versions and releases all eight public Reed and NecrassRs packages together:

- `reed-parser`
- `reed-compiler`
- `necrassrs`
- `necrassrs-build`
- `necrassrs-http`
- `necrassrs-axum`
- `necrassrs-actix`
- `necrassrs-cli`

These packages inherit `workspace.package.version`. Their local dependencies use both `path` and `version`, so one manifest supports workspace development and crates.io publication. Example packages remain unpublished.

Release-plz opens a release pull request after changes reach `main`. That pull request updates the shared version, workspace dependency requirements, and changelog. The repository sets `release_always = false`, so an ordinary push to `main` does not authorize publication.

Review a release pull request for:

- the intended shared version;
- API compatibility findings from `cargo-semver-checks`;
- accurate package-specific changelog entries;
- updated path-and-version dependency requirements; and
- successful formatting, tests, linting, documentation, packaging, and generated-consumer checks.

Non-conventional commit messages produce patch increments by default. Correct the proposed version when the commit history does not express the intended compatibility boundary. Automated API checks do not detect every SemVer violation.

## Check package archives

Inspect every public archive before publication:

```sh
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo package --workspace --exclude axum-server --exclude actix-server --exclude tutorial --locked --no-verify
```

Use `cargo package --list -p <package>` to inspect an individual archive. The workspace command skips archive verification because a package that depends on another unpublished workspace package cannot complete registry-backed verification. Independently packageable archives can be verified before the initial release:

```sh
cargo package --locked -p reed-parser
cargo package --locked -p necrassrs-http
cargo package --locked -p necrassrs-cli
```

The remaining archive verifications become release gates after their dependencies are available from crates.io.

## Publish the initial release

The initial publication is supervised and sequential because crates.io trusted publishing cannot create a package's first release. Verify package-name control and publishing permissions before starting. Never record registry credentials in repository files or release evidence.

Publish and verify packages in this order:

1. Publish `reed-parser`, then build a fresh registry-only parser consumer.
2. Verify and publish `reed-compiler`, then build a fresh registry-only compiler consumer.
3. Verify and publish `necrassrs`, `necrassrs-build`, and `necrassrs-http`.
4. Verify and publish `necrassrs-axum` and `necrassrs-actix`.
5. Replace the CLI templates' temporary Git dependencies with the released registry version, run the generated-consumer checks, and publish `necrassrs-cli` last.

For each package, inspect `cargo publish --dry-run -p <package>` before running `cargo publish -p <package>`. Wait until crates.io resolves a newly published dependency before verifying or publishing its dependents. If a step fails, stop and resume from the first unpublished package after correcting the release state; published versions cannot be overwritten.

External verification must use a fresh directory and lockfile without path dependencies, Git dependencies, `[patch]` entries, or source replacement. Record the published versions, source revision, crates.io and docs.rs links, dependency source metadata, and executed checks.

After all initial releases exist, configure a crates.io trusted publisher for every public package and add the release-plz `release` job with `id-token: write`. Merging a reviewed release pull request then authorizes release-plz to publish the shared version, create package tags, and create GitHub releases.
