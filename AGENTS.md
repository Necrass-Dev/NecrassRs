# Agent instructions

These instructions apply to AI agents working in the NecrassRs repository. See [AI_POLICY.md](AI_POLICY.md) for disclosure and contributor responsibilities, and the [contributing guide](docs/src/content/docs/docs/guide/contributing.md#package-boundaries) for the current package layout.

## Before making changes

- Read the current request, relevant issue, the documents above, and the affected code.
- Check `git status` and preserve existing changes made by other contributors.
- Distinguish the target architecture from the current implementation. Do not assume planned crates or APIs already exist.
- Before fixing a bug, trace callers and related paths. Address shared causes at the appropriate boundary.
- Verify public APIs and dependency behavior against source code or official documentation for the applicable version.

## Scope and authority

- Follow explicit scope and restrictions in the current conversation. Do not edit files during a review-only task.
- When implementation or fixes are requested, proceed with authorized investigation, edits, and validation without repeatedly asking about routine implementation choices.
- Do not silently settle unresolved public contracts for implementation convenience. Explain consequential decisions and ask only for information needed to resolve them.
- When a change in design is explicitly requested, update the relevant documentation. Do not quietly rewrite the design to justify a workaround.
- Documentation describes current behavior, not a permanent restriction on internal design. Preserve public contracts and user data when refactoring; explain compatibility changes and update affected tests and guidance when changing those contracts.
- Use an issue's specified branch when applicable. Do not damage existing work through checkout, reset, or overwrites.
- Permission to edit local files does not itself authorize remote publication, merging, package releases, or messages to other people. Perform those actions only within the authorized scope.

## Implementation boundaries

- SDL defines the public GraphQL contract. Do not synchronize Rust implementations back into SDL.
- Reuse existing schema models and validation rather than duplicating them. Reed currently supplies these facilities; NecrassRs implements runtime execution. Package and module boundaries may change with the work being implemented.
- Applications own their Context types and construction. Avoid introducing framework, CLI, or build-tool dependencies into runtime code without a concrete requirement.
- Preserve retained resolver bodies and unrelated user code during source synchronization. Do not remove application-owned implementations automatically. Consult [Organizing Resolver Files](docs/src/content/docs/docs/manual/resolver-files.md) for current generated names, ownership rules, and limitations; changes to public names and synchronization behavior require compatibility review.
- Initialization must not overwrite existing files or symbolic links.
- Check existing code, the standard library, and existing dependencies first. Do not add abstractions, configuration, crates, or dependencies without a concrete requirement.
- Do not remove input validation, error handling, or user data protection to simplify an implementation.
- Do not handle invalid external input or ordinary domain errors with panics. Keep the deliberate process-exit policy for unimplemented resolvers separate.
- Maintain the root `Cargo.lock`. Avoid unrelated dependency updates.

## Validation

Start with the smallest meaningful check of the changed behavior. Leave a runnable regression check for nontrivial behavior changes and bug fixes. Test contracts rather than mirroring the implementation.

For Rust code changes, use these baseline commands from the repository root. Run focused package or test checks first when useful.

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

- Compile generated code when changing the generator. String snapshots alone do not establish that the user contract works.
- For execution changes, check affected input states, list nullability, error paths, null propagation, field selection, and mutation ordering.
- Check thread and lifetime contracts at compile time, including the entire execution future's `Send` bound and request-scoped borrowing.
- Test process termination in a separate executable process. Do not assume a library's Cargo profile propagates to consumer applications.
- Do not hide failures by accepting incorrect results or disabling tests.
- For documentation-only changes, check links, claims, and formatting; unrelated Rust tests are not required.
- Report checks that could not run and why. Do not present historical or prototype results as validation of the current change.

## Language, review, and reporting

Official repository documents must be written in English, including policies, architecture documents, READMEs, and contribution guides. Follow the user's requested language in conversation. Keep code, comments, documents, and other repository artifacts professional and free of conversational roleplay.

The website groups its pages under `docs/src/content/docs/docs/` to match the sidebar: `introduction/`, `guide/`, and `manual/`. Keep step-by-step server setup in the tutorial and type mappings and `GraphQLInput` semantics in the Types manual. Maintain current user and contributor guidance in the relevant documentation pages. Track proposals, remaining work, and historical validation results in issues and pull requests rather than separate archive documents. Update repository references when removing or moving documentation.

In reviews, lead with reproducible defects and risks, identifying locations and impact. Distinguish findings from hypotheses.

Summarize changes, validation commands and results, unverified behavior, and unresolved decisions concisely.

Do not copy private neighboring repositories, conversation transcripts, credentials, or personal filesystem paths into public artifacts. Include only the design facts that can be shared publicly.

Do not use prefix like `feat:` for `docs:` in commit messages.

## Reference

The organization of these instructions was informed by [Gelite's AGENTS.md](https://github.com/gelite-dev/gelite/blob/main/AGENTS.md) and adapted to NecrassRs. External documents are references; changes to them do not automatically change this repository's rules.
