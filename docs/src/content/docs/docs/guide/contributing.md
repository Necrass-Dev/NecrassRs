---
title: "Contributing"
description: Develop and validate changes to Reed and NecrassRs.
---

NecrassRs welcomes bug reports, design discussions, documentation improvements, and code contributions. The repository's [contributing guide](https://github.com/Necrass-Dev/NecrassRs/blob/main/CONTRIBUTING.md) is the canonical source for development, changelog, versioning, release, and recovery procedures. Before changing the repository, also read the relevant issue, [architecture](https://github.com/Necrass-Dev/NecrassRs/blob/main/archives/architecture.md), and [AI usage policy](https://github.com/Necrass-Dev/NecrassRs/blob/main/AI_POLICY.md).

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

## Changelogs and releases

Contributors record user-visible changes as Sacho fragments. Maintainers select
versions and prepare releases explicitly; ordinary contribution pull requests
and branch pushes do not publish packages. Follow the canonical
[contributing guide](https://github.com/Necrass-Dev/NecrassRs/blob/main/CONTRIBUTING.md)
for fragment commands, exemptions, version selection, tag-triggered publishing,
and partial-publication recovery.

Describe public API changes and compatibility risks clearly in the contribution
pull request. Do not rely on automated SemVer checks to communicate a breaking
change. When a change affects packaged files or manifest metadata, inspect the
affected archive before review:

```sh
cargo package --list -p <package>
cargo package -p <package> --locked
```
