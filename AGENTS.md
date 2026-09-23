# Agent instructions

These instructions apply to AI agents working in the NecrassRs repository. See [AI_POLICY.md](AI_POLICY.md) for disclosure and contributor responsibilities, and [docs/architecture.md](docs/architecture.md) for product architecture and design decisions.

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
- Use an issue's specified branch when applicable. Do not damage existing work through checkout, reset, or overwrites.
- Permission to edit local files does not itself authorize remote publication, merging, package releases, or messages to other people. Perform those actions only within the authorized scope.

## Implementation boundaries

- SDL defines the public GraphQL contract. Do not synchronize Rust implementations back into SDL.
- Use Apollo Compiler's models and validation. Do not duplicate its schema model or validator. Choosing an execution engine remains a separate decision.
- Separate code generation, Cargo and filesystem integration, runtime execution, the Axum adapter, and the CLI as described in the architecture document.
- Keep Axum, CLI, and build-tool dependencies out of the execution core. Applications own their Context types and construction.
- Place disposable contracts and dispatch in `OUT_DIR`. Build integration creates and synchronizes the designated resolver implementation in `src` using the SDL-to-Rust naming rules: preserve retained method bodies and unrelated user code, add explicit stubs, and delete removed methods. A rename is deletion plus addition. Follow `docs/architecture.md` section 7.3; do not interpret user-code preservation as a ban on synchronizing SDL-owned declarations.
- Initialization must not overwrite existing files or symbolic links.
- Do not copy prototype code. Implement from the observed behavior and validation scenarios.
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

In reviews, lead with reproducible defects and risks, identifying locations and impact. Distinguish findings from hypotheses.

Summarize changes, validation commands and results, unverified behavior, and unresolved decisions concisely.

Do not copy private neighboring repositories, conversation transcripts, credentials, or personal filesystem paths into public artifacts. Include only the design facts that can be shared publicly.

## Reference

The organization of these instructions was informed by [Gelite's AGENTS.md](https://github.com/gelite-dev/gelite/blob/main/AGENTS.md) and adapted to NecrassRs. External documents are references; changes to them do not automatically change this repository's rules.
