#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "[error] Usage: $0 <path-to-binary>"
  exit 1
fi

BINARY="$1"

if [ ! -f "${BINARY}" ]; then
  echo "[error] Binary not found: ${BINARY}"
  exit 1
fi

echo "[run] Smoke test: ${BINARY} --version"
"${BINARY}" --version

echo "[ok] Smoke test passed"
