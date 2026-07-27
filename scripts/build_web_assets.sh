#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
WEB_DIR="${REPO_ROOT}/apps/web"
DIST_DIR="${WEB_DIR}/dist"
CRATE_WEB_ASSETS_DIR="${REPO_ROOT}/rust/apps/memorph-cli/assets/web"

if [ ! -f "${WEB_DIR}/package.json" ]; then
  echo "[error] Missing frontend package: ${WEB_DIR}/package.json"
  exit 1
fi

echo "[run] Building React frontend"
npm --prefix "${WEB_DIR}" run build

if [ ! -f "${DIST_DIR}/index.html" ]; then
  echo "[error] Frontend build did not produce ${DIST_DIR}/index.html"
  exit 1
fi

rm -rf "${CRATE_WEB_ASSETS_DIR}"
mkdir -p "${CRATE_WEB_ASSETS_DIR}"
cp -R "${DIST_DIR}/." "${CRATE_WEB_ASSETS_DIR}/"

echo "[ok] Synced frontend assets to ${CRATE_WEB_ASSETS_DIR}"
