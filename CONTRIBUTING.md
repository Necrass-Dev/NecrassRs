# Contributing to NecrassRs

NecrassRs welcomes bug reports, design discussions, documentation improvements,
and code contributions. Before changing the repository, read the relevant issue,
[package boundaries](docs/src/content/docs/docs/guide/contributing.md#package-boundaries), and
[`AI_POLICY.md`](AI_POLICY.md). Use an issue's specified branch when it has one,
and keep unrelated changes in separate commits.

## Validate changes

Start with the smallest check that exercises the changed behavior. Rust changes
must pass the workspace baseline before review:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Run the documentation checks after changing the website:

```sh
cd docs
pnpm install --frozen-lockfile
pnpm lint
pnpm fmt:check
pnpm check
pnpm build
```

Bug fixes and nontrivial behavior changes need a runnable regression check.
Generator changes must also compile a generated consumer. Report checks that
could not run and do not replace a failing assertion with an incorrect
expectation.

## Write changelog fragments

### Set up Sacho

The repository pins Sacho in [`mise.toml`](mise.toml). Install the repository
tools and confirm the version from the repository root:

```sh
mise install
sacho init --no-interactive
sacho --version
```

Initialization leaves the tracked project files intact and registers Sacho's
changelog merge driver in the local Git configuration.

Run commands from the repository root. If mise is not activated in your shell,
use `mise exec -- sacho <command>`. Follow the repository pin rather than a
globally installed version. See the upstream
[everyday workflow](https://sacho.dev/guide/everyday-workflow),
[sections](https://sacho.dev/concepts/sections), and
[command reference](https://sacho.dev/reference/commands).

| File | Responsibility |
| --- | --- |
| `sacho.toml` | Fragment locations, issue links, sections, and coverage policy. |
| `changes.d/**/*.md` | Editable source entries for the next release. |
| `changes.d/next` | Maintainer-selected next version; managed with `sacho next`. |
| `CHANGELOG.md` | Generated unreleased output and permanent released history. |

Fragments are the source of truth. Do not hand-edit the generated unreleased
region in `CHANGELOG.md`; edit the fragments and regenerate it instead.
Released sections are permanent.

### Choose the crate section

Use one changelog section per public crate, with the crate name as both the
section ID and the displayed heading. All eight sections belong to the same
release version; they do not introduce separate release schedules.

| Section ID | Fragment directory | User-facing changes |
| --- | --- | --- |
| `necrassrs` | `changes.d/necrassrs/` | Execution, resolver contracts, inputs, outputs, and subscriptions. |
| `necrassrs-build` | `changes.d/necrassrs-build/` | SDL code generation, Cargo integration, and resolver synchronization. |
| `necrassrs-http` | `changes.d/necrassrs-http/` | Shared HTTP media-type negotiation and transport utilities. |
| `necrassrs-axum` | `changes.d/necrassrs-axum/` | Axum request extraction, responses, and GraphiQL integration. |
| `necrassrs-actix` | `changes.d/necrassrs-actix/` | Actix Web request extraction, responses, and GraphiQL integration. |
| `necrassrs-cli` | `changes.d/necrassrs-cli/` | The `necrass` command, starter templates, installation, and CLI binaries. |
| `reed-compiler` | `changes.d/reed-compiler/` | Schema and executable models, validation, and compiler APIs. |
| `reed-parser` | `changes.d/reed-parser/` | GraphQL parsing, syntax trees, and parser diagnostics. |

These eight sections are explicitly configured in `sacho.toml`, in the order
shown above. Select the exact crate name when adding or previewing entries;
omitting `--section` or using an unknown name is an error:

```sh
sacho add --section necrassrs-cli prebuilt-binaries
sacho preview --section necrassrs-cli
```

This creates `changes.d/necrassrs-cli/prebuilt-binaries.md`. Section IDs are
crate names, not categories such as `Added` or `Fixed`. Examples and the
documentation website do not get separate sections; describe user-visible
changes under the affected public crate.

For changes spanning crates, write the relevant user impact in each affected
crate's fragment. For example, describe a shared HTTP negotiation fix under
`necrassrs-http` and any changed adapter behavior under the corresponding
adapter. Do not copy the same paragraph into every section or invent entries
for crates whose internal implementation changed without a user-visible effect.

Section `paths` rules associate source changes with the required section during
coverage checks; they do not choose a section automatically for `sacho add`.
A fragment in another section cannot satisfy a section-specific requirement.
Each crate's manifest and `src/` are covered, along with the CLI templates.
The existing repository-wide `[check].paths` rules remain enabled; changes
to the root `Cargo.toml` can be covered by a fragment in any relevant section.
Use the exemption described below for commits with no user-visible effect.

### Write for users

Name each fragment after the change, not an issue or pull request number. A
fragment contains exactly one top-level unordered list; related changes may
share a fragment. Start entries with a verb such as `Added`, `Changed`,
`Deprecated`, `Fixed`, or `Removed`. Describe the behavior users receive and
what they need to do when upgrading. Include public API signatures and migration
instructions when they change. For generated APIs, identify the generated Rust
contract and explain the corresponding resolver implementation change.

For example, a fragment can describe a behavior and link to its issue:

```markdown
 -  Added GET query support to the Axum adapter. Register the GraphQL handler
    for GET as well as POST to enable it. [[#30]]
```

Include the relevant issue or PR references. The `#` shortcut is defined in
`sacho.toml`; Sacho expands it to an issue link. Keep the fragment with the
implementation in every covered commit. When a later commit changes the
behavior, update the existing fragment in that commit rather than adding a
duplicate entry.

Do not record implementation steps or intermediate designs that do not ship.
If an unreleased feature is removed, remove its entry and run `sacho sync`.

### Format, preview, and check

Format, inspect, and validate fragments before committing:

```sh
sacho fmt
sacho preview
sacho check
```

`fmt` formats fragments and refreshes the materialized unreleased region.
Review both kinds of changes. `preview` prints the upcoming release without
writing files; `show <version>` reads an already released section instead.
Use `sync` when only the generated region is stale. Inspect hand edits before
using `sync --force`, which permits discarding them. `check --fix` repairs
mechanical formatting and synchronization problems, not missing explanations.

Stage the implementation, its fragments, and the generated changelog together,
then run `sacho check --staged`. Only index contents count for staged coverage;
an unstaged fragment does not cover a staged source change.

| Check | Purpose |
| --- | --- |
| `sacho check` | Validate fragments, formatting, links, the next-version file, and generated output. |
| `sacho check --staged` | Also enforce coverage for the staged Git changes. |
| `sacho check --base <base-sha>` | Also enforce coverage separately for each commit after the PR base. |

### Changes without a fragment

Documentation-only, test-only, and internal changes with no user-visible
behavior change do not require a fragment.
When a covered source change has no user-visible effect, add this trailer to the
commit message:

```text
Changelog: none
```

CI runs `sacho check --base` commit by commit. An exemption applies only to the
commit containing the trailer. `sacho check --staged` cannot apply exemptions
because the commit message does not exist yet. Adding a fragment in a later
commit does not cover an earlier undocumented commit. Use the same base SHA as
the PR check and ensure the local history contains that revision.

## Select versions

The eight public Reed and NecrassRs packages use one workspace version and are
released together, including unchanged packages. Maintainers select each
version explicitly; commit messages and API compatibility tools do not select
it.

Set a maintainer-selected stable `major.minor.patch` version without a `v`
prefix using Bash, jq, Cargo, and the pinned Sacho:

```sh
bash .github/scripts/set-version.sh <version>
```

The script updates the workspace version, internal dependency requirements,
CLI templates, root lockfile, and Sacho's next-version label. Examples inherit
their internal dependencies from the workspace. Existing released changelog
sections and unrelated locked dependencies are preserved. `sacho next` alone
does not update Cargo metadata.

Cargo runs offline; populate its cache with `cargo fetch --locked` first if
needed. Repeating the same version validates the state without rewriting it.
Invalid input, conflicting versions, and symbolic-link destinations are
rejected. Command failures restore the affected files; concurrent edits are
reported rather than overwritten during rollback. A forced termination or
power loss can interrupt a multi-file update, so inspect the diff before
retrying after such an interruption. Failed updates report a temporary backup
directory for recovery.

The script does not finalize release notes, commit, tag, or publish. Its
disposable-workspace regression checks run with:

```sh
bash .github/scripts/set-version-test.sh
```

During `0.x` development:

- New user-facing capabilities and incompatible changes increment the minor
  version.
- Compatible fixes and improvements increment the patch version.

Examples remain `publish = false` but their internal dependency requirements
must match the prepared workspace version.

## Prepare a release

Prepare releases in a pull request. The tagged source must already contain the
complete release; CI does not change versions or commit files.

**Known limitation:** Finalizing a release deletes consumed fragments, which
can make the current PR coverage check reject their earlier commits despite
the finalized notes. [Issue #89](https://github.com/Necrass-Dev/NecrassRs/issues/89)
tracks this separately from section adoption. Do not bypass the failure by
disabling coverage, exempting user-visible commits, or restoring duplicate
unreleased notes.

1. Choose the release version and run
   `bash .github/scripts/set-version.sh <version>`.
2. Review the version changes, including the workspace manifest, CLI templates,
   `Cargo.lock`, and the generated unreleased changelog heading.
3. Review `sacho preview` as a user upgrading from the previous release. Remove
   implementation history, combine duplicate entries, and make migrations
   explicit.
4. Finalize the notes with `sacho release <version> --date YYYY-MM-DD`. Commit
   the released changelog, deletion of the consumed fragments and
   `changes.d/next`, and version changes together. An explicit version must
   agree with `changes.d/next` when that file contains a version. Always supply
   the reviewed release date instead of relying on the local calendar.
5. Run the workspace baseline and the release checks:

   ```sh
   .github/scripts/release-test.sh
   .github/scripts/release.sh verify <version>
   ```

6. Inspect the finalized notes using the publication rendering:

   ```sh
   sacho show <version> --skip-heading --no-word-wrap
   ```

   This removes automatic line wrapping from the export, preserves intentional
   Markdown hard breaks, and does not rewrite the stored changelog. Confirm
   that the release pull request contains no unrelated changes.

Without `--next`, `sacho release` closes the unreleased region. Start the next
development cycle separately with a maintainer-selected `sacho next <version>`
after tagging; do not automatically select the next version. A subsequent
release with no substantive entries requires an intentional `--allow-empty`.

Do not create or push the release tag before the preparation pull request is
merged.

## Publish a release

After the preparation pull request is merged, tag its merge commit with the
bare stable version and push that tag. Do not add a `v` prefix or use per-crate
tags.

```sh
git tag <version> <merge-commit>
git push origin <version>
```

`.github/workflows/release-plz.yml` validates the tagged commit, obtains a
short-lived crates.io token through Trusted Publishing, publishes all eight
packages in dependency order, and waits for each required package to become
available. After every package is available, it creates one GitHub Release from
`sacho show <version>`.

Verify the workflow, all eight crates.io package versions, the tag target, and
the single GitHub Release body. The release body must match the finalized Sacho
notes.

## Recover a partial publication

Do not move, replace, or force-push a release tag. Investigate the failed job,
then rerun that workflow run. The release script verifies package versions that
already exist, skips them, and resumes with the first unpublished package.
Registry lookup failures stop the workflow rather than being interpreted as a
successful publication.

The GitHub Release is created only after all packages are available. A rerun
keeps an existing release when its body matches the finalized Sacho notes and
fails if the body differs. Resolve configuration, authentication, registry, or
notes discrepancies before rerunning; never attempt to overwrite a published
crate version.
