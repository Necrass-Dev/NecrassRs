#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
readonly RELEASE_SCRIPT="$SCRIPT_DIR/release.sh"
TEMPORARY=$(mktemp -d)
readonly TEMPORARY
readonly FAKE_BIN="$TEMPORARY/bin"
readonly FIXTURE_ROOT="$TEMPORARY/repository"
readonly STATE="$TEMPORARY/state"
readonly LOG="$TEMPORARY/publish.log"

cleanup() {
  rm -rf "$TEMPORARY"
}
trap cleanup EXIT

mkdir -p "$FAKE_BIN" "$FIXTURE_ROOT/crates/necrassrs-cli/templates/actix" \
  "$FIXTURE_ROOT/crates/necrassrs-cli/templates/axum" "$STATE"

cat >"$FIXTURE_ROOT/crates/necrassrs-cli/templates/actix/manifest.toml" <<'EOF'
necrassrs = "0.2.0"
necrassrs-actix = "0.2.0"
necrassrs-build = "0.2.0"
EOF
cat >"$FIXTURE_ROOT/crates/necrassrs-cli/templates/axum/manifest.toml" <<'EOF'
necrassrs = "0.2.0"
necrassrs-axum = "0.2.0"
necrassrs-build = "0.2.0"
EOF

cat >"$TEMPORARY/metadata.json" <<'EOF'
{"packages":[
  {"name":"reed-parser","version":"0.2.0","publish":null,"dependencies":[]},
  {"name":"necrassrs-http","version":"0.2.0","publish":null,"dependencies":[]},
  {"name":"necrassrs-cli","version":"0.2.0","publish":null,"dependencies":[]},
  {"name":"reed-compiler","version":"0.2.0","publish":null,"dependencies":[{"name":"reed-parser","path":"/reed-parser","req":"^0.2.0"}]},
  {"name":"necrassrs","version":"0.2.0","publish":null,"dependencies":[{"name":"reed-compiler","path":"/reed-compiler","req":"^0.2.0"}]},
  {"name":"necrassrs-build","version":"0.2.0","publish":null,"dependencies":[{"name":"reed-compiler","path":"/reed-compiler","req":"^0.2.0"}]},
  {"name":"necrassrs-actix","version":"0.2.0","publish":null,"dependencies":[{"name":"necrassrs","path":"/necrassrs","req":"^0.2.0"},{"name":"necrassrs-http","path":"/necrassrs-http","req":"^0.2.0"}]},
  {"name":"necrassrs-axum","version":"0.2.0","publish":null,"dependencies":[{"name":"necrassrs","path":"/necrassrs","req":"^0.2.0"},{"name":"necrassrs-http","path":"/necrassrs-http","req":"^0.2.0"}]},
  {"name":"example","version":"0.2.0","publish":[],"dependencies":[{"name":"necrassrs","path":"/necrassrs","req":"^0.2.0"}]}
]}
EOF

cat >"$FAKE_BIN/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  metadata) cat "$TEST_METADATA" ;;
  publish)
    package=
    while (($#)); do
      if [[ "$1" == "--package" ]]; then
        package=$2
        break
      fi
      shift
    done
    [[ -n "$package" ]]
    echo "$package" >>"$TEST_LOG"
    touch "$TEST_STATE/crate-$package"
    ;;
  *) exit 64 ;;
esac
EOF

cat >"$FAKE_BIN/sacho" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=
while (($#)); do
  if [[ "$1" == "--output-file" ]]; then
    output=$2
    break
  fi
  shift
done
[[ -n "$output" ]]
if [[ -n "${TEST_EMPTY_NOTES:-}" ]]; then
  : >"$output"
else
  printf 'Released without application-facing changes.\n' >"$output"
fi
EOF

cat >"$FAKE_BIN/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=
url=
proto=
proto_redir=
while (($#)); do
  case "$1" in
    --output) output=$2; shift 2 ;;
    --proto) proto=$2; shift 2 ;;
    --proto-redir) proto_redir=$2; shift 2 ;;
    http*) url=$1; shift ;;
    *) shift ;;
  esac
done
[[ -n "$output" && -n "$url" && "$proto" == '=https' && "$proto_redir" == '=https' ]]
if [[ "$url" == *api.github.com* ]]; then
  if [[ -f "$TEST_STATE/release" ]]; then
    printf '{"body":"Released without application-facing changes.\\n"}\n' >"$output"
    printf 200
  else
    printf '{}\n' >"$output"
    printf 404
  fi
  exit
fi
package=${url%/*}
package=${package##*/}
if [[ "${TEST_REGISTRY_FAILURE:-}" == "$package" ]]; then
  printf '{}\n' >"$output"
  printf 503
elif [[ -f "$TEST_STATE/crate-$package" ]]; then
  printf '{"version":{"crate":"%s","num":"0.2.0"}}\n' "$package" >"$output"
  printf 200
else
  printf '{}\n' >"$output"
  printf 404
fi
EOF

cat >"$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo release >>"$TEST_STATE/release-calls"
touch "$TEST_STATE/release"
EOF

chmod +x "$FAKE_BIN/cargo" "$FAKE_BIN/curl" "$FAKE_BIN/gh" "$FAKE_BIN/sacho"

export PATH="$FAKE_BIN:$PATH"
export RELEASE_ROOT="$FIXTURE_ROOT"
export TEST_METADATA="$TEMPORARY/metadata.json"
export TEST_STATE="$STATE"
export TEST_LOG="$LOG"
export GH_TOKEN=test-token

"$RELEASE_SCRIPT" validate 0.2.0
if "$RELEASE_SCRIPT" validate v0.2.0 >/dev/null 2>&1; then
  echo "invalid tag was accepted" >&2
  exit 1
fi
if "$RELEASE_SCRIPT" validate necrassrs-v0.2.0 >/dev/null 2>&1; then
  echo "legacy package tag was accepted" >&2
  exit 1
fi

jq '(.packages[0].version) = "0.1.2"' "$TEMPORARY/metadata.json" >"$TEMPORARY/version-mismatch.json"
export TEST_METADATA="$TEMPORARY/version-mismatch.json"
if "$RELEASE_SCRIPT" validate 0.2.0 >/dev/null 2>&1; then
  echo "version mismatch was accepted" >&2
  exit 1
fi

jq '(.packages[3].dependencies[0].req) = "^0.1.2"' "$TEMPORARY/metadata.json" >"$TEMPORARY/dependency-mismatch.json"
export TEST_METADATA="$TEMPORARY/dependency-mismatch.json"
if "$RELEASE_SCRIPT" validate 0.2.0 >/dev/null 2>&1; then
  echo "dependency mismatch was accepted" >&2
  exit 1
fi

export TEST_METADATA="$TEMPORARY/metadata.json"
export TEST_EMPTY_NOTES=1
if "$RELEASE_SCRIPT" validate 0.2.0 >/dev/null 2>&1; then
  echo "missing release notes were accepted" >&2
  exit 1
fi
unset TEST_EMPTY_NOTES

touch "$STATE/crate-reed-parser" "$STATE/crate-necrassrs-http" "$STATE/crate-reed-compiler"
"$RELEASE_SCRIPT" publish 0.2.0

cat >"$TEMPORARY/expected.log" <<'EOF'
necrassrs-cli
necrassrs
necrassrs-build
necrassrs-actix
necrassrs-axum
EOF
cmp "$TEMPORARY/expected.log" "$LOG"
[[ $(wc -l <"$STATE/release-calls") -eq 1 ]]

"$RELEASE_SCRIPT" publish 0.2.0
cmp "$TEMPORARY/expected.log" "$LOG"
[[ $(wc -l <"$STATE/release-calls") -eq 1 ]]

rm -f "$STATE/crate-necrassrs-axum"
export TEST_REGISTRY_FAILURE=necrassrs-axum
if "$RELEASE_SCRIPT" publish 0.2.0 >/dev/null 2>&1; then
  echo "registry failure was accepted" >&2
  exit 1
fi

echo "release regression checks passed"
