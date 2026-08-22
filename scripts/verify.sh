#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

pnpm install --frozen-lockfile
pnpm run format:check
git diff --check
pnpm run check
pnpm run lint
pnpm run test
pnpm run build
pnpm run e2e:smoke
pnpm run e2e
pnpm run check:duplication
pnpm run coverage
