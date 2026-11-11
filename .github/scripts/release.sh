#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
readonly ROOT="${RELEASE_ROOT:-$(cd -- "$SCRIPT_DIR/../.." && pwd)}"
readonly USER_AGENT="NecrassRs-release/1.0 (https://github.com/Necrass-Dev/NecrassRs)"
readonly PACKAGES=(
  reed-parser
  necrassrs-http
  necrassrs-cli
  reed-compiler
  necrassrs
  necrassrs-build
  necrassrs-actix
  necrassrs-axum
)
TEMPORARY=$(mktemp -d)
readonly TEMPORARY
trap 'rm -rf "$TEMPORARY"' EXIT

die() {
  echo "release: $*" >&2
  exit 1
}

require_command() {
  local dependency=$1
  command -v "$dependency" >/dev/null || die "required command not found: $dependency"
}

validate_tag() {
  local tag=$1
  [[ "$tag" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] ||
    die "tag must be a stable version without a v prefix: $tag"
}

validate_metadata() {
  local tag=$1 metadata expected actual mismatch bad_dependencies
  metadata=$(cargo metadata --format-version 1 --no-deps --locked)

  expected=$(printf '%s\n' "${PACKAGES[@]}" | sort)
  actual=$(jq -r '.packages[] | select(.publish != []) | .name' <<<"$metadata" | sort)
  [[ "$actual" == "$expected" ]] || die "public workspace packages do not match the release set"

  mismatch=$(jq -r --arg version "$tag" '.packages[] | select(.version != $version) | "\(.name) \(.version)"' <<<"$metadata")
  [[ -z "$mismatch" ]] || die "workspace package versions do not match $tag: $mismatch"

  bad_dependencies=$(jq -r --arg version "^$tag" '
    .packages[] as $package
    | $package.dependencies[]
    | select(.path != null and .req != $version)
    | "\($package.name) -> \(.name) \(.req)"
  ' <<<"$metadata")
  [[ -z "$bad_dependencies" ]] ||
    die "internal dependency requirements do not match $tag: $bad_dependencies"
}

validate_templates() {
  local tag=$1 template package
  for template in "$ROOT"/crates/necrassrs-cli/templates/{actix,axum}/manifest.toml; do
    for package in necrassrs necrassrs-build; do
      grep -Fqx "$package = \"$tag\"" "$template" ||
        die "$template does not use $package $tag"
    done
  done
  grep -Fqx "necrassrs-actix = \"$tag\"" "$ROOT/crates/necrassrs-cli/templates/actix/manifest.toml" ||
    die "Actix template does not use necrassrs-actix $tag"
  grep -Fqx "necrassrs-axum = \"$tag\"" "$ROOT/crates/necrassrs-cli/templates/axum/manifest.toml" ||
    die "Axum template does not use necrassrs-axum $tag"
}

write_release_notes() {
  local tag=$1 output=$2
  sacho show "$tag" --skip-heading --no-word-wrap --output-file "$output"
  [[ -s "$output" ]] || die "release notes for $tag are empty"
}

validate_release() {
  local tag=$1 notes
  validate_tag "$tag"
  validate_metadata "$tag"
  validate_templates "$tag"
  notes="$TEMPORARY/validation-notes.md"
  write_release_notes "$tag" "$notes"
}

verify_packages() {
  local tag=$1 package archive package_dir
  validate_release "$tag"
  cargo package --workspace --exclude axum-server --exclude actix-server --exclude tutorial \
    --locked --no-verify --allow-dirty

  mkdir -p "$TEMPORARY/packages" "$TEMPORARY/consumer/src"

  for package in "${PACKAGES[@]}"; do
    archive="$ROOT/target/package/$package-$tag.crate"
    [[ -f "$archive" ]] || die "missing package archive: $archive"
    tar -xzf "$archive" -C "$TEMPORARY/packages"
  done

  cat >"$TEMPORARY/consumer/Cargo.toml" <<EOF
[package]
name = "necrassrs-release-consumer"
version = "0.0.0"
edition = "2024"

[dependencies]
reed-parser = "=$tag"
reed-compiler = "=$tag"
necrassrs = "=$tag"
necrassrs-http = "=$tag"
necrassrs-actix = "=$tag"
necrassrs-axum = "=$tag"

[build-dependencies]
necrassrs-build = "=$tag"

[patch.crates-io]
EOF
  for package in "${PACKAGES[@]}"; do
    [[ "$package" == necrassrs-cli ]] && continue
    package_dir="$TEMPORARY/packages/$package-$tag"
    printf '"%s" = { path = "%s" }\n' "$package" "$package_dir" >>"$TEMPORARY/consumer/Cargo.toml"
  done
  printf 'fn main() {}\n' >"$TEMPORARY/consumer/src/main.rs"
  printf 'fn main() {}\n' >"$TEMPORARY/consumer/build.rs"

  cargo generate-lockfile --manifest-path "$TEMPORARY/consumer/Cargo.toml"
  cargo check --manifest-path "$TEMPORARY/consumer/Cargo.toml" --locked

  cargo generate-lockfile --manifest-path "$TEMPORARY/packages/necrassrs-cli-$tag/Cargo.toml"
  cargo check --manifest-path "$TEMPORARY/packages/necrassrs-cli-$tag/Cargo.toml" --locked
}

registry_status() {
  local package=$1 version=$2 output=$3 status
  status=$(curl --silent --show-error --location --proto '=https' --proto-redir '=https' \
    --user-agent "$USER_AGENT" \
    --output "$output" \
    --write-out '%{http_code}' \
    "https://crates.io/api/v1/crates/$package/$version") ||
    die "failed to query crates.io for $package $version"
  case "$status" in
    200)
      jq -e --arg package "$package" --arg version "$version" \
        '.version.crate == $package and .version.num == $version' "$output" >/dev/null ||
        die "crates.io returned an invalid response for $package $version"
      return 0
      ;;
    404) return 1 ;;
    *) die "crates.io returned HTTP $status for $package $version" ;;
  esac
}

wait_for_package() {
  local package=$1 version=$2 output
  output="$TEMPORARY/registry-$package.json"
  for _ in {1..30}; do
    if registry_status "$package" "$version" "$output"; then
      return
    fi
    sleep 10
  done
  die "$package $version did not become available on crates.io"
}

publish_packages() {
  local tag=$1 package output
  output="$TEMPORARY/registry.json"
  for package in "${PACKAGES[@]}"; do
    if registry_status "$package" "$tag" "$output"; then
      echo "$package $tag is already published"
      continue
    fi
    cargo publish --package "$package" --locked --no-verify
    wait_for_package "$package" "$tag"
  done
}

github_release_status() {
  local tag=$1 output=$2 status repository=${GITHUB_REPOSITORY:-Necrass-Dev/NecrassRs}
  [[ -n "${GH_TOKEN:-}" ]] || die "GH_TOKEN is required to query GitHub releases"
  status=$(curl --silent --show-error --location --proto '=https' --proto-redir '=https' \
    --user-agent "$USER_AGENT" \
    --header "Accept: application/vnd.github+json" \
    --header "Authorization: Bearer $GH_TOKEN" \
    --header "X-GitHub-Api-Version: 2022-11-28" \
    --output "$output" \
    --write-out '%{http_code}' \
    "https://api.github.com/repos/$repository/releases/tags/$tag") ||
    die "failed to query GitHub Release $tag"
  case "$status" in
    200) return 0 ;;
    404) return 1 ;;
    *) die "GitHub returned HTTP $status for release $tag" ;;
  esac
}

create_release() {
  local tag=$1 notes response existing asset
  shift
  notes="$TEMPORARY/release-notes.md"
  response="$TEMPORARY/github-release.json"
  write_release_notes "$tag" "$notes"

  if github_release_status "$tag" "$response"; then
    existing=$(jq -r '.body' "$response")
    [[ "$existing" == "$(cat "$notes")" ]] || die "existing GitHub Release $tag has different notes"
    echo "GitHub Release $tag already exists"
    for asset in "$@"; do
      if jq -e --arg name "${asset##*/}" '.assets[] | select(.name == $name)' "$response" >/dev/null; then
        jq -e --arg name "${asset##*/}" \
          '.assets[] | select(.name == $name) | .state == "uploaded" and .size > 0' "$response" >/dev/null ||
          die "existing release asset is incomplete: ${asset##*/}"
        echo "Release asset ${asset##*/} already exists; preserving it"
      else
        gh release upload "$tag" "$asset"
      fi
    done
    # gh release create can leave a draft if an asset upload fails.
    if jq -e '.draft == true' "$response" >/dev/null; then
      gh release edit "$tag" --draft=false
    fi
    return
  fi

  gh release create "$tag" --verify-tag --title "$tag" --notes-file "$notes" "$@"
}

main() {
  local command=${1:-} tag=${2:-}
  [[ -n "$command" && -n "$tag" && $# -eq 2 ]] ||
    die "usage: $0 <validate|verify|publish> <version>"
  cd "$ROOT"
  for dependency in cargo curl jq sacho; do
    require_command "$dependency"
  done

  case "$command" in
    validate) validate_release "$tag" ;;
    verify) verify_packages "$tag" ;;
    publish)
      require_command gh
      validate_release "$tag"
      [[ -n "${RELEASE_ASSETS_DIR:-}" ]] || die "RELEASE_ASSETS_DIR is required for publication"
      local prefix="$RELEASE_ASSETS_DIR/necrass-$tag" asset
      local assets=(
        "$prefix-x86_64-unknown-linux-gnu.tar.gz"
        "$prefix-x86_64-apple-darwin.tar.gz"
        "$prefix-aarch64-apple-darwin.tar.gz"
        "$prefix-x86_64-pc-windows-msvc.zip"
      )
      for asset in "${assets[@]}"; do
        [[ -f "$asset" && -s "$asset" ]] || die "missing or empty CLI archive: $asset"
      done
      publish_packages "$tag"
      create_release "$tag" "${assets[@]}"
      ;;
    *) die "unknown command: $command" ;;
  esac
}

main "$@"
