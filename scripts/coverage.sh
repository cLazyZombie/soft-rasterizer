#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

cargo +nightly llvm-cov \
  --workspace \
  --all-features \
  --all-targets \
  --show-missing-lines \
  --fail-under-lines 100
