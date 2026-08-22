#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

if [[ -n "${NOSE_BIN:-}" ]]; then
  NOSE_BINARY="$NOSE_BIN"
elif command -v nose >/dev/null 2>&1; then
  NOSE_BINARY="$(command -v nose)"
else
  echo "nose is required. Run: brew install corca-ai/tap/nose" >&2
  exit 1
fi

if [[ ! -x "$NOSE_BINARY" ]]; then
  echo "nose binary is not executable: $NOSE_BINARY" >&2
  exit 1
fi

if ! ACTUAL_VERSION="$("$NOSE_BINARY" --version 2>&1)"; then
  echo "failed to read nose version from: $NOSE_BINARY" >&2
  exit 1
fi
if [[ ! "$ACTUAL_VERSION" =~ ^nose[[:space:]][0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "binary does not identify as nose: $NOSE_BINARY ($ACTUAL_VERSION)" >&2
  exit 1
fi

if [[ -n "${SOFT_RASTERIZER_SKIP_NOSE_VERSION_CHECK:-}" ]]; then
  echo "warning: SOFT_RASTERIZER_SKIP_NOSE_VERSION_CHECK is set; skipping latest version check" >&2
elif ! command -v brew >/dev/null 2>&1; then
  echo "warning: Homebrew is unavailable; using local $ACTUAL_VERSION" >&2
elif ! LATEST_NOSE_VERSION="$(
  brew info --json=v2 corca-ai/tap/nose |
    node -e '
      let input = "";
      process.stdin.setEncoding("utf8");
      process.stdin.on("data", (chunk) => { input += chunk; });
      process.stdin.on("end", () => {
        const version = JSON.parse(input).formulae?.[0]?.versions?.stable;
        if (typeof version !== "string" || !/^[0-9]+\.[0-9]+\.[0-9]+$/.test(version)) {
          process.exit(1);
        }
        process.stdout.write(version);
      });
    '
)"; then
  echo "warning: latest nose lookup failed; using local $ACTUAL_VERSION" >&2
elif [[ "$ACTUAL_VERSION" != "nose $LATEST_NOSE_VERSION" ]]; then
  echo "latest nose $LATEST_NOSE_VERSION is required, but found: $ACTUAL_VERSION" >&2
  echo "Upgrade nose with: brew upgrade corca-ai/tap/nose" >&2
  exit 1
fi

cd "$REPO_ROOT"
QUERY_ROOTS=()
for SOURCE_ROOT in renderer-core/src renderer-core/tests renderer-wasm/src renderer-wasm/tests; do
  if [[ -d "$SOURCE_ROOT" ]]; then
    QUERY_ROOTS+=(--root "$SOURCE_ROOT")
  fi
done
if [[ ${#QUERY_ROOTS[@]} -eq 0 ]]; then
  echo "No owned Rust source roots found." >&2
  exit 1
fi

"$NOSE_BINARY" query \
  "${QUERY_ROOTS[@]}" \
  --config "$REPO_ROOT/nose.toml" \
  --baseline "$REPO_ROOT/.nose-baseline.json" \
  --ignore-file "$REPO_ROOT/nose.ignore.json" \
  'lines>7' \
  'shared>5' \
  top=0 \
  all \
  --format json |
  node "$SCRIPT_DIR/check_nose_result.mjs"
