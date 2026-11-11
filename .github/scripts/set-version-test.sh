#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPOSITORY=$(cd -- "$SCRIPT_DIR/../.." && pwd)
TEMPORARY=$(mktemp -d)
trap 'rm -rf "$TEMPORARY"' EXIT
export RELEASE_ROOT="$TEMPORARY/workspace"
mkdir -p "$RELEASE_ROOT/changes.d" "$TEMPORARY/bin"
cd "$RELEASE_ROOT"
packages=(reed-parser reed-compiler necrassrs necrassrs-build necrassrs-http necrassrs-axum necrassrs-actix necrassrs-cli)
cat > Cargo.toml <<'MANIFEST'
[workspace]
members = ["crates/*"]
exclude = ["third-party"]
resolver = "3"
[workspace.package]
version = "1.2.3"
[workspace.dependencies]
# Keep example version 1.2.3.
third-party = { path = "third-party", version = "1.2.3" }
MANIFEST
for package in "${packages[@]}"; do
  mkdir -p "crates/$package/src"
  touch "crates/$package/src/lib.rs"
  printf '[package]\nname = "%s"\nversion.workspace = true\nedition = "2024"\n' "$package" > "crates/$package/Cargo.toml"
  printf '%s = { path = "crates/%s", version = "1.2.3" }\n' "$package" "$package" >> Cargo.toml
done
cat >> crates/necrassrs-cli/Cargo.toml <<'MANIFEST'
[dependencies]
necrassrs.workspace = true
third-party.workspace = true
MANIFEST
mkdir -p third-party/src
touch third-party/src/lib.rs
printf '[package]\nname = "third-party"\nversion = "1.2.3"\n[workspace]\n' > third-party/Cargo.toml
for framework in actix axum; do
  mkdir -p "crates/necrassrs-cli/templates/$framework"
  cat > "crates/necrassrs-cli/templates/$framework/manifest.toml" <<MANIFEST
[package]
name = "{{name}}"
version = "0.1.0"
[dependencies]
necrassrs = "1.2.3"
necrassrs-$framework = "1.2.3"
[build-dependencies]
necrassrs-build = "1.2.3"
MANIFEST
done
cp "$REPOSITORY/sacho.toml" sacho.toml
cat > CHANGELOG.md <<'CHANGELOG'
Changelog
=========

Version 0.9.0
-------------

Released on January 1, 2020.

 -  Documented version 1.2.3.
CHANGELOG
mkdir -p changes.d/necrassrs-cli
printf ' -  Added a version update fixture.\n' > changes.d/necrassrs-cli/version-test.md
sacho sync
cargo generate-lockfile --offline
cp -R "$RELEASE_ROOT" "$TEMPORARY/original"
run() { bash "$SCRIPT_DIR/set-version.sh" "$@"; }
reset_fixture() {
  cd "$TEMPORARY"
  rm -rf "$RELEASE_ROOT"
  cp -R "$TEMPORARY/original" "$RELEASE_ROOT"
  cd "$RELEASE_ROOT"
}
reject_unchanged() {
  cp -R "$RELEASE_ROOT" "$TEMPORARY/before"
  if run "$@" > "$TEMPORARY/output" 2>&1; then
    echo "unexpected success: $*" >&2; exit 1
  fi
  diff -ru "$TEMPORARY/before" "$RELEASE_ROOT"
  rm -rf "$TEMPORARY/before"
}
for version in v1.2.4 1.2 01.2.4 1.2.4-rc.1 $'1.2.4\n' 0.9.0; do
  reject_unchanged "$version"
done
reject_unchanged
reject_unchanged 1.2.4 extra
printf '2.0.0\n' > changes.d/next
reject_unchanged 1.2.4
reset_fixture
sed 's/necrassrs = "1.2.3"/necrassrs = "9.9.9"/' crates/necrassrs-cli/templates/axum/manifest.toml > "$TEMPORARY/template"
cp "$TEMPORARY/template" crates/necrassrs-cli/templates/axum/manifest.toml
reject_unchanged 1.2.4
reset_fixture
mv Cargo.lock "$TEMPORARY/linked-lock"
ln -s "$TEMPORARY/linked-lock" Cargo.lock
reject_unchanged 1.2.4
reset_fixture

sacho next 1.2.4
run 1.2.4
cargo metadata --no-deps --format-version 1 --locked --offline | jq -e 'all(.packages[]; .version == "1.2.4")' >/dev/null
grep -Fqx 'third-party = { path = "third-party", version = "1.2.3" }' Cargo.toml
grep -Fqx '# Keep example version 1.2.3.' Cargo.toml
grep -Fqx '1.2.4' changes.d/next
grep -Fqx 'Version 1.2.4' CHANGELOG.md
cmp changes.d/necrassrs-cli/version-test.md "$TEMPORARY/original/changes.d/necrassrs-cli/version-test.md"
sed -n '/^Version 0.9.0/,$p' CHANGELOG.md > "$TEMPORARY/history-after"
sed -n '/^Version 0.9.0/,$p' "$TEMPORARY/original/CHANGELOG.md" > "$TEMPORARY/history-before"
cmp "$TEMPORARY/history-before" "$TEMPORARY/history-after"
for framework in actix axum; do
  template="crates/necrassrs-cli/templates/$framework/manifest.toml"
  grep -Fqx 'version = "0.1.0"' "$template"
  for package in necrassrs necrassrs-build "necrassrs-$framework"; do
    grep -Fqx "$package = \"1.2.4\"" "$template"
  done
done
cp -R "$RELEASE_ROOT" "$TEMPORARY/updated"
run 1.2.4
diff -ru "$TEMPORARY/updated" "$RELEASE_ROOT"

# The selected version must remain usable by the real release and export commands.
sacho release 1.2.4 --date 2026-11-11 --no-resolve-links
sacho check
[[ ! -e changes.d/next ]]
[[ ! -e changes.d/necrassrs-cli/version-test.md ]]
sacho show 1.2.4 --skip-heading --no-word-wrap --output-file "$TEMPORARY/release-notes.md"
grep -Fq 'Released on November 11, 2026.' "$TEMPORARY/release-notes.md"
grep -Fq '### necrassrs-cli' "$TEMPORARY/release-notes.md"
grep -Fq 'Added a version update fixture.' "$TEMPORARY/release-notes.md"
if grep -Fq 'Version 1.2.4' "$TEMPORARY/release-notes.md"; then
  echo 'Release export must omit the version heading.' >&2; exit 1
fi
sed -n '/^Version 0.9.0/,$p' CHANGELOG.md > "$TEMPORARY/history-after"
cmp "$TEMPORARY/history-before" "$TEMPORARY/history-after"

export REAL_CARGO=$(command -v cargo)
export REAL_SACHO=$(command -v sacho)
cat > "$TEMPORARY/bin/cargo" <<'MOCK'
#!/usr/bin/env bash
if [[ "$1" == update ]]; then
  if [[ "$FAILURE" == cargo ]]; then
    echo broken > Cargo.lock
    exit 1
  fi
  "$REAL_CARGO" "$@" || exit $?
  if [[ "$FAILURE" == unrelated-lock ]]; then
    echo '# Unexpected lockfile edit' >> Cargo.lock
  fi
else
  exec "$REAL_CARGO" "$@"
fi
MOCK
cat > "$TEMPORARY/bin/sacho" <<'MOCK'
#!/usr/bin/env bash
if [[ "$1" == next && "$FAILURE" == sacho ]]; then
  echo broken > CHANGELOG.md
  echo broken > changes.d/next
  exit 1
fi
exec "$REAL_SACHO" "$@"
MOCK
chmod +x "$TEMPORARY/bin/"*
export PATH="$TEMPORARY/bin:$PATH"
for FAILURE in cargo sacho unrelated-lock; do
  export FAILURE
  reset_fixture
  reject_unchanged 1.2.4
  backup=$(sed -n 's/.*Backups: \(.*\)\/before/\1/p' "$TEMPORARY/output")
  [[ -n "$backup" ]] && rm -rf -- "$backup"
done
echo 'Version update tests passed.'
