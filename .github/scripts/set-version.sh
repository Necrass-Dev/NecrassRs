#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
ROOT=${RELEASE_ROOT:-$(cd -- "$SCRIPT_DIR/../.." && pwd)}
cd -- "$ROOT"

die() { echo "set-version: $*" >&2; exit 1; }
[[ $# == 1 ]] || die "usage: bash .github/scripts/set-version.sh <major.minor.patch>"
version=$1
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] ||
  die "use a stable major.minor.patch version without a v prefix"
for command in cargo jq sacho; do
  command -v "$command" >/dev/null || die "required command not found: $command"
done
files=(Cargo.toml Cargo.lock CHANGELOG.md changes.d/next
  crates/necrassrs-cli/templates/actix/manifest.toml
  crates/necrassrs-cli/templates/axum/manifest.toml)
for file in "${files[@]}"; do
  path=$file
  while [[ "$path" != . ]]; do
    [[ ! -L "$path" ]] || die "refusing symbolic link: $path"
    path=$(dirname -- "$path")
  done
  if [[ "$file" == changes.d/next && ! -e "$file" ]]; then continue; fi
  [[ -f "$file" ]] || die "required regular file is missing: $file"
done

TEMPORARY=$(mktemp -d)
writing=false
complete=false
# shortcut: rollback covers command failures, not power loss; add a journal if crash recovery is needed.
cleanup() {
  local status=$? file
  trap - EXIT
  if $writing && ! $complete; then
    for file in "${files[@]}"; do
      if [[ -L "$file" ]] || { [[ -f "$TEMPORARY/expected/$file" ]] && ! cmp -s "$file" "$TEMPORARY/expected/$file"; } ||
        { [[ ! -f "$TEMPORARY/expected/$file" ]] && [[ -e "$file" ]]; }; then
        echo "set-version: cannot restore changed file: $file (backup: $TEMPORARY/before)" >&2
        status=1
        continue
      fi
      if [[ -f "$TEMPORARY/before/$file" ]]; then
        cp -p "$TEMPORARY/before/$file" "$file" || status=1
      else
        rm -f -- "$file" || status=1
      fi
    done
    echo "set-version: update failed; inspect the diff. Backups: $TEMPORARY/before" >&2
  else
    rm -rf -- "$TEMPORARY"
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
for file in "${files[@]}"; do
  mkdir -p "$TEMPORARY/before/$(dirname "$file")" "$TEMPORARY/expected/$(dirname "$file")"
  if [[ -f "$file" ]]; then
    cp -p "$file" "$TEMPORARY/before/$file"
    cp -p "$file" "$TEMPORARY/expected/$file"
  fi
done
metadata() { cargo metadata --no-deps --format-version 1 --locked --offline; }
metadata > "$TEMPORARY/metadata.json"
current=$(jq -er '.packages[] | select(.name == "necrassrs") | .version' "$TEMPORARY/metadata.json")
names=$(jq -r '[.packages[].name] | join(" ")' "$TEMPORARY/metadata.json")
validate_metadata() {
  jq -e --arg version "$1" '
    [.packages[].name] as $names |
    all(.packages[]; .version == $version and
      all(.dependencies[]; if .path != null and (.name as $name | $names | index($name)) != null
        then .req == ("^" + $version) else true end))
  ' "$TEMPORARY/metadata.json" >/dev/null || die "workspace versions or internal requirements are inconsistent"
}
validate_metadata "$current"
while IFS= read -r manifest; do
  grep -Eq '^version\.workspace[[:space:]]*=[[:space:]]*true$' "$manifest" ||
    die "package must inherit the workspace version: $manifest"
done < <(jq -r '.packages[].manifest_path' "$TEMPORARY/metadata.json")
if [[ "$version" != "$current" ]] && awk -v version="$version" '
  /^Version / { selected=($0 == "Version " version) }
  selected && /^Released on / { found=1 }
  END { exit !found }
' CHANGELOG.md; then
  die "version $version is already released"
fi
if [[ -f changes.d/next ]]; then
  next=$(cat changes.d/next)
  [[ "$next" == "$current" || "$next" == "$version" ]] || die "changes.d/next conflicts with the requested version"
fi

# Only rewrite version declarations owned by this workspace, never matching prose or external dependencies.
awk -v old="$current" -v new="$version" -v names="$names" '
  BEGIN { n=split(names, list, " "); for(i=1;i<=n;i++) member[list[i]]=1 }
  /^\[/ { section=$0 }
  {
    key=$1
    if ((section == "[workspace.package]" && key == "version") ||
        (section == "[workspace.dependencies]" && key in member)) {
      if (!index($0, "version = \"" old "\"")) exit 1
      sub(/version = "[^"]*"/, "version = \"" new "\"")
      if (key == "version") count++
    }
    print
  }
  END { if (count != 1) exit 1 }
' Cargo.toml > "$TEMPORARY/manifest" || die "unexpected workspace version declaration"
for framework in actix axum; do
  file="crates/necrassrs-cli/templates/$framework/manifest.toml"
  awk -v old="$current" -v new="$version" -v adapter="necrassrs-$framework" '
    $1 == "necrassrs" || $1 == "necrassrs-build" || $1 == adapter {
      if ($0 != $1 " = \"" old "\"") exit 1
      seen[$1]++; $0=$1 " = \"" new "\""
    }
    { print }
    END { if (seen["necrassrs"] != 1 || seen["necrassrs-build"] != 1 || seen[adapter] != 1) exit 1 }
  ' "$file" > "$TEMPORARY/$framework" || die "unexpected template version declaration: $file"
done
sacho check
if [[ "$version" == "$current" ]]; then
  echo "Version $version is already set."
  complete=true
  exit 0
fi

external_lock() {
  awk -v names="$names" '
    BEGIN { n=split(names,list," "); for(i=1;i<=n;i++) member[list[i]]=1 }
    function flush() { if (name != "" && (source || !(name in member))) printf "%s", block }
    /^\[\[package\]\]/ { flush(); block=""; name=""; source=0 }
    /^name = / { name=$3; gsub(/"/, "", name) }
    /^source = / { source=1 }
    { block=block $0 "\n" }
    END { flush() }
  ' "$1"
}
external_lock Cargo.lock > "$TEMPORARY/external-before"
for file in "${files[@]}"; do
  if [[ -f "$TEMPORARY/before/$file" ]]; then
    cmp -s "$file" "$TEMPORARY/before/$file" || die "file changed during preparation: $file"
  else
    [[ ! -e "$file" ]] || die "file appeared during preparation: $file"
  fi
done
writing=true
cp "$TEMPORARY/manifest" "$TEMPORARY/expected/Cargo.toml"
cp "$TEMPORARY/manifest" Cargo.toml
for framework in actix axum; do
  file="crates/necrassrs-cli/templates/$framework/manifest.toml"
  cp "$TEMPORARY/$framework" "$TEMPORARY/expected/$file"
  cp "$TEMPORARY/$framework" "$file"
done
status=0
cargo update --workspace --offline || status=$?
cp Cargo.lock "$TEMPORARY/expected/Cargo.lock"
[[ $status == 0 ]] || die "Cargo update failed"
external_lock Cargo.lock > "$TEMPORARY/external-after"
cmp -s "$TEMPORARY/external-before" "$TEMPORARY/external-after" || die "Cargo changed unrelated locked dependencies"
metadata > "$TEMPORARY/metadata.json"
validate_metadata "$version"
sacho next "$version" || status=$?
for file in CHANGELOG.md changes.d/next; do
  if [[ -f "$file" ]]; then cp "$file" "$TEMPORARY/expected/$file"; fi
done
[[ $status == 0 ]] || die "Sacho next failed"
sacho check
complete=true
echo "Set version $current -> $version. Review the diff before committing."
