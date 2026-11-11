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
| `sacho.toml` | Fragment locations, issue links, sections, and validation settings. |
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
`sacho.toml`; Sacho expands it to an issue link. Include the fragment with the
implementation in the pull request. When a later commit changes the
behavior, update the existing fragment rather than adding a
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

Stage the implementation, its fragments, and the generated changelog together.
CI runs `sacho check` to validate fragments, formatting, links, the next-version
file, and generated output. Automatic missing-fragment coverage is disabled.
Reviewers check that user-visible changes have release notes in the appropriate
crate sections and that compatibility changes include migration guidance.

### Changes without a fragment

Documentation-only, test-only, and internal changes with no user-visible
behavior change do not require a fragment or a commit-message exemption trailer.

## Select versions

The eight public Reed and NecrassRs packages use one workspace version and are
released together, including unchanged packages. Maintainers select each
version explicitly; commit messages and API compatibility tools do not select
it.

For a release, enter a stable `major.minor.patch` version without a `v` prefix
in the Release workflow described below. CI uses the existing version-update
script. To inspect version changes locally with Bash, jq, Cargo, and the pinned
Sacho, run:

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

The version-update script alone does not finalize release notes, commit, tag,
or publish.

During `0.x` development:

- New user-facing capabilities and incompatible changes increment the minor
  version.
- Compatible fixes and improvements increment the patch version.

Examples remain `publish = false` but their internal dependency requirements
must match the prepared workspace version.

## Prepare a release

Review and merge the changes and changelog fragments before starting a release.
Read `sacho preview` as a user upgrading from the previous release: remove
implementation history, combine duplicate entries, and make migrations explicit.

In GitHub, open **Actions → Release → Run workflow**, select **main**, enter
the version (for example, `2.0.0`), and run the workflow. The equivalent CLI is:

```sh
gh workflow run release-plz.yml --ref main -f version=2.0.0
```

The workflow must be present on the default branch before it can be dispatched.
Only dispatches from `main` in the upstream repository can publish. Branch and
tag rules must allow the workflow token to push the preparation commit and tag.

For a new version, `.github/scripts/release-ci.sh prepare` updates versions with
`set-version.sh` and finalizes the changelog with Sacho using the current UTC
date. It consumes the fragments and closes the unreleased region without
selecting another version. Empty subsequent releases are rejected by Sacho;
the workflow does not silently allow them or rewrite an already released section.

CI then runs formatting, Clippy, the full workspace tests, and
`release.sh verify`. Only after those checks succeed does it
create `Prepare release <version>` and a bare `<version>` tag on that commit.
It pushes `main` and the tag atomically: either both refs are updated or neither
is. If `main` advances during preparation or validation, the run stops without
publishing; review the new changes and start a new run from `main`.

Preparation failures leave only the disposable runner checkout changed. No
preparation commit or tag is pushed before verification succeeds. Do not run
the CI helper in a working checkout: it switches revisions, commits, and pushes.

## Publish a release

After pushing the prepared commit and tag, the same workflow builds CLI archives
from that exact commit for Linux x86_64, macOS x86_64 and ARM64, and Windows
x86_64. Each build extracts its archive and checks the CLI version and project
initialization on its native runner. All four builds must succeed before
publication starts. See the [installation guide](docs/src/content/docs/docs/guide/installation.md#prebuilt-binaries)
for archive names and the supported platform matrix.

The publish job downloads the archives, obtains a short-lived crates.io token
through Trusted Publishing, and checks that all four archives are present and
nonempty before publishing any package. It publishes all eight packages in
dependency order and waits for each required package to become available.
After every package is available, it
creates one GitHub Release with the CLI archives and notes from
`sacho show <version> --skip-heading --no-word-wrap`.
The export preserves intentional Markdown hard breaks without adding automatic
line wrapping or rewriting the stored changelog.

GitHub automatically computes SHA-256 digests for uploaded release assets and
displays them beside the files and in the API. We use those digests instead of
publishing a separate checksum file. Actions artifacts only transfer archives
between jobs; the GitHub Release attachments are the public downloads.

Publication continues in the same workflow because a tag pushed with
`GITHUB_TOKEN` does not trigger a separate push workflow. Manually pushing a
tag no longer starts publication. The workflow file remains `release-plz.yml`
to preserve the filename used by existing crates.io Trusted Publisher settings;
renaming it requires updating those settings for all eight packages.

Verify the workflow, all eight crates.io package versions, the tag target,
the single GitHub Release body, and all four CLI attachments and their digests.
The release body must match the finalized Sacho notes.

## Recover a partial publication

Do not move, replace, or force-push a release tag. Investigate the failed job,
then rerun that workflow run or dispatch the Release workflow from `main` with
the same version. When the tag already exists, preparation checks out its
immutable commit, validates it, and preserves its release date and notes.
Later changes on `main` are not included. The tag must belong to `main` history.
The workflow preserves its release automation outside the source checkout so
that recovery also works for tags predating the current scripts.

The release script verifies package versions that already exist, skips them,
and resumes with the first unpublished package. Registry lookup failures stop
the workflow rather than being interpreted as a successful publication.

The GitHub Release is created only after all packages are available. A rerun
keeps an existing release when its body matches the finalized Sacho notes and
fails if the body differs. Existing uploaded CLI assets are preserved, and only
missing files are uploaded; rebuilds never replace previously published bytes
or their digests. An incomplete existing asset stops recovery; inspect and
remove that failed upload before retrying. If an interrupted creation left a
draft release, recovery publishes it after all expected assets are uploaded.
Resolve configuration, authentication, registry, or notes discrepancies before
rerunning; never attempt to overwrite a published crate version.
