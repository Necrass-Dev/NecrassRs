---
title: "Contributing"
description: Develop and validate changes to Reed and NecrassRs.
---

NecrassRs welcomes bug reports, design discussions, documentation improvements, and code contributions. The repository's [contributing guide](https://github.com/Necrass-Dev/NecrassRs/blob/main/CONTRIBUTING.md) is the canonical source for development, changelog, versioning, release, and recovery procedures. Before changing the repository, also read the relevant issue and [AI usage policy](https://github.com/Necrass-Dev/NecrassRs/blob/main/AI_POLICY.md).

## Package boundaries

The following map describes the current workspace. Internal responsibilities and
test locations can move as the implementation evolves; changes to public APIs,
generated names, or source ownership need compatibility and migration guidance.

SDL defines the public GraphQL contract. Cargo builds generate disposable Rust
contracts and dispatch in `OUT_DIR` and synchronize editable resolver declarations
in `src`. Rust implementations do not update SDL.

| Package                             | Responsibility                                                                                         |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `reed-parser`                       | GraphQL tokens and concrete syntax trees.                                                              |
| `reed-compiler`                     | Schema and executable models, validation, introspection, and variable coercion.                        |
| `necrassrs`                         | Request preparation, field-argument processing, resolver dispatch, execution, and response completion. |
| `necrassrs-build`                   | SDL discovery, code generation, Cargo rebuild tracking, and resolver source synchronization.           |
| `necrassrs-http`                    | Framework-independent response media negotiation.                                                      |
| `necrassrs-axum`, `necrassrs-actix` | Native HTTP extraction, response conversion, and GraphiQL helpers.                                     |
| `necrassrs-cli`                     | The `necrass` initializer and starter templates.                                                       |

NecrassRs uses Reed's models and validation. The build library separates code
generation from Cargo and filesystem integration, and the execution core has no
build, CLI, or HTTP framework dependencies. Applications own their Context,
handlers, and server lifecycle. The CLI initializes projects;
subsequent builds call the build library without requiring an installed CLI.

Source synchronization preserves retained resolver bodies and unrelated user code.
See [Organizing Resolver Files](/docs/resolver-files/) for declaration ownership,
module discovery, and recovery limits, and [Subscriptions](/docs/subscriptions/)
for stream ownership and cancellation.

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

Parsing, validation, introspection, and input-coercion rule tests currently live
in Reed. NecrassRs tests cover integration, execution, generated consumers, Cargo,
and filesystem behavior. Place regressions with the behavior they exercise.
Cross-layer checks should verify the integration boundary
without duplicating Reed's entire rule matrix or diagnostic text. Use the
[specification baseline](/docs/reed/#specification-baseline) when adding regressions;
track remaining conformance work and observed validation results in issues and
pull requests rather than maintaining a second test-result ledger.

## Changelogs and releases

Contributors record user-visible changes as Sacho fragments. Maintainers select
versions and prepare releases explicitly; ordinary contribution pull requests
and branch pushes do not publish packages. Follow the canonical
[contributing guide](https://github.com/Necrass-Dev/NecrassRs/blob/main/CONTRIBUTING.md)
for fragment commands, changelog review, version selection, tag-triggered publishing,
and partial-publication recovery.

Describe public API changes and compatibility risks clearly in the contribution
pull request. Do not rely on automated SemVer checks to communicate a breaking
change. When a change affects packaged files or manifest metadata, inspect the
affected archive before review:

```sh
cargo package --list -p <package>
cargo package -p <package> --locked
```
