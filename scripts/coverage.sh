#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

pnpm run check:coverage-policy

if [[ -n "${LCOV_FILTER_BIN:-}" ]]; then
  LCOV_FILTER_BINARY="$LCOV_FILTER_BIN"
elif command -v lcov_filter >/dev/null 2>&1; then
  LCOV_FILTER_BINARY="$(command -v lcov_filter)"
else
  echo "lcov_filter is required. Run: cargo install --git https://github.com/cLazyZombie/lcov_filter --force" >&2
  exit 1
fi

if [[ ! -x "$LCOV_FILTER_BINARY" ]]; then
  echo "lcov_filter binary is not executable: $LCOV_FILTER_BINARY" >&2
  exit 1
fi
if ! LCOV_FILTER_HELP="$("$LCOV_FILTER_BINARY" --help 2>&1)"; then
  echo "failed to read lcov_filter help from: $LCOV_FILTER_BINARY" >&2
  exit 1
fi
if [[ "$LCOV_FILTER_HELP" != *"--marker-file"* ]]; then
  echo "lcov_filter with LCOV_EXCL_FILE support is required: $LCOV_FILTER_BINARY" >&2
  echo "Upgrade with: cargo install --git https://github.com/cLazyZombie/lcov_filter --force" >&2
  exit 1
fi

cargo +nightly llvm-cov \
  --workspace \
  --all-features \
  --all-targets \
  --lcov \
  --quiet |
  "$LCOV_FILTER_BINARY" --text
