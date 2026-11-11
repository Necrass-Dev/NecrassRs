#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd -- "${RELEASE_ROOT:-$SCRIPT_DIR/../..}"

die() { echo "release-ci: $*" >&2; exit 1; }
[[ $# == 2 && ( "$1" == prepare || "$1" == tag ) ]] ||
  die "usage: bash .github/scripts/release-ci.sh <prepare|tag> <version>"
command=$1
version=$2
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] ||
  die "use a stable major.minor.patch version without a v prefix"
[[ "${GITHUB_REF:-}" == refs/heads/main ]] || die "run the release workflow from main"

require_clean() {
  [[ -z "$(git status --porcelain)" ]] || die "release checkout contains unexpected changes"
}

if [[ "$command" == prepare ]]; then
  require_clean
  git fetch origin refs/heads/main:refs/remotes/origin/main --tags
  if git show-ref --verify --quiet "refs/tags/$version"; then
    git merge-base --is-ancestor "refs/tags/$version" refs/remotes/origin/main ||
      die "existing release tag is not part of main"
    git checkout --detach "refs/tags/$version"
    bash "$SCRIPT_DIR/release.sh" validate "$version"
  else
    [[ "$(git rev-parse HEAD)" == "$(git rev-parse refs/remotes/origin/main)" ]] ||
      die "main changed; start a new workflow run from main"
    bash "$SCRIPT_DIR/set-version.sh" "$version"
    sacho release "$version" --date "$(date -u +%F)"
    bash "$SCRIPT_DIR/release.sh" validate "$version"
  fi
  sacho check
  exit 0
fi

# The workflow runs all release checks between preparation and this step.
bash "$SCRIPT_DIR/release.sh" validate "$version"
sacho check
if git show-ref --verify --quiet "refs/tags/$version"; then
  require_clean
  [[ "$(git rev-parse HEAD)" == "$(git rev-parse "refs/tags/$version^{commit}")" ]] ||
    die "checkout differs from the existing release tag"
  git fetch origin "refs/tags/$version"
  [[ "$(git rev-parse HEAD)" == "$(git rev-parse 'FETCH_HEAD^{commit}')" ]] ||
    die "remote release tag differs from the checkout"
  echo "Resuming release $version from its existing tag."
  exit 0
fi

git fetch origin refs/heads/main:refs/remotes/origin/main
[[ "$(git rev-parse HEAD)" == "$(git rev-parse refs/remotes/origin/main)" ]] ||
  die "main changed during validation; start a new workflow run from main"
git diff --cached --quiet || die "validation staged unexpected changes"
git add -- Cargo.toml Cargo.lock CHANGELOG.md changes.d \
  crates/necrassrs-cli/templates/actix/manifest.toml \
  crates/necrassrs-cli/templates/axum/manifest.toml
git diff --quiet && [[ -z "$(git ls-files --others --exclude-standard)" ]] ||
  die "validation changed files outside release metadata"
git diff --cached --quiet && die "release preparation produced no changes"
git commit -m "Prepare release $version"
require_clean
git tag "$version"
# Reject concurrent main updates and publish both refs together, without force.
git push --atomic origin HEAD:refs/heads/main "refs/tags/$version"
