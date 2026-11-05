# Contributing to NecrassRs

NecrassRs welcomes bug reports, design discussions, documentation improvements,
and code contributions. Before changing the repository, read the relevant issue,
[`archives/architecture.md`](archives/architecture.md), and
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

The repository pins Sacho in [`mise.toml`](mise.toml). Install the repository
tools and confirm the version from the repository root:

```sh
mise install
sacho init --no-interactive
sacho --version
```

Initialization leaves the tracked project files intact and registers Sacho's
changelog merge driver in the local Git configuration.

Create one fragment for each user-visible change, using a short topic name
rather than an issue or pull request number:

```sh
sacho add clearer-errors
```

Edit the generated Markdown file in `changes.d/`. A fragment contains exactly
one top-level unordered list. Describe the behavior that users will receive and
what they need to do when upgrading. Include public API signatures and migration
instructions when they change. For generated APIs, identify the generated Rust
contract and explain the corresponding resolver implementation change.

Do not record implementation steps or intermediate designs that do not ship. If
a change evolves before release, update its existing fragment. If the
user-visible change is removed, delete its fragment and run `sacho sync`.

Format, inspect, and validate fragments before committing:

```sh
sacho fmt
sacho preview
sacho check
sacho check --staged
```

Documentation-only, test-only, and internal changes do not require a fragment.
When a covered source change has no user-visible effect, add this trailer to the
commit message:

```text
Changelog: none
```

CI runs `sacho check --base` commit by commit. An exemption applies only to the
commit containing the trailer. `sacho check --staged` cannot apply exemptions
because the commit message does not exist yet.

Do not edit the materialized unreleased section in `CHANGELOG.md` directly.
Edit its fragments and run `sacho sync` instead. Released sections are permanent.

## Select versions

The eight public Reed and NecrassRs packages use one workspace version and are
released together, including unchanged packages. Maintainers select each
version explicitly; commit messages and API compatibility tools do not select
it.

During `0.x` development:

- New user-facing capabilities and incompatible changes increment the minor
  version.
- Compatible fixes and improvements increment the patch version.

Examples remain `publish = false` but their internal dependency requirements
must match the prepared workspace version.

## Prepare a release

Prepare releases in a pull request. The tagged source must already contain the
complete release; CI does not change versions or commit files.

1. Choose the release version and set it with `sacho next <version>`.
2. Update the workspace version, every internal `path + version` requirement,
   example dependencies, CLI template dependencies, affected package examples,
   and `Cargo.lock`.
3. Review `sacho preview` as a user upgrading from the previous release. Remove
   implementation history, combine duplicate entries, and make migrations
   explicit.
4. Finalize the notes with `sacho release <version> --date YYYY-MM-DD`. Commit
   the released changelog and consumed fragments with the version changes.
5. Run the workspace baseline and the release checks:

   ```sh
   .github/scripts/release-test.sh
   .github/scripts/release.sh verify <version>
   ```

6. Confirm that `sacho show <version>` prints the finalized notes and that the
   release pull request contains no unrelated changes.

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

For the first release under this process, merge the implementation pull request,
close issue #86, tag that merge commit as `0.2.0`, push the tag, and verify all
eight `0.2.0` packages and the `0.2.0` GitHub Release.

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
