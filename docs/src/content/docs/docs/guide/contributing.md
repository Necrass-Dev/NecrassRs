---
title: "Contributing"
description: Develop and validate changes to Reed and NecrassRs.
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

## Releases and versioning

The eight public Reed and NecrassRs packages share one workspace version and
release together. Contributors normally do not edit the workspace version,
package dependency requirements, changelog, release workflow, or release-plz
configuration. Include those files only when an issue explicitly changes the
release process.

After a contribution reaches `main`, release-plz prepares a separate release
pull request. Maintainers review and merge that pull request to publish the
coordinated release. An ordinary contribution pull request does not publish a
package.

Describe public API changes and compatibility risks clearly in the contribution
pull request. Do not rely on automated SemVer checks to communicate a breaking
change. When a change affects packaged files or manifest metadata, inspect the
affected archive before review:

```sh
cargo package --list -p <package>
cargo package -p <package> --locked
```
