#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TAURI_DIR="${REPO_ROOT}/desktop/tauri"
TARGET="${1:-aarch64-apple-darwin}"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "[error] macOS DMG builds require Darwin"
  exit 1
fi

VERSION="$(python3 - "${REPO_ROOT}" <<'PY'
import json
import sys
from pathlib import Path

config = json.loads(Path(sys.argv[1]).joinpath("desktop/tauri/tauri.conf.json").read_text(encoding="utf-8"))
print(config["version"])
PY
)"

case "${TARGET}" in
  aarch64-apple-darwin) MARKER="aarch64" ;;
  x86_64-apple-darwin) MARKER="x64" ;;
  universal-apple-darwin) MARKER="universal" ;;
  *) MARKER="${TARGET}" ;;
esac

echo "[run] Building macOS app bundle for ${TARGET}"
(
  cd "${TAURI_DIR}"
  cargo tauri build --bundles app --target "${TARGET}" --ci
)

APP_ROOT="${TAURI_DIR}/target/${TARGET}/release/bundle/macos"
APP_PATH="${APP_ROOT}/memorph.app"
if [ ! -d "${APP_PATH}" ]; then
  echo "[error] Missing app bundle: ${APP_PATH}"
  exit 1
fi

DMG_DIR="${TAURI_DIR}/target/${TARGET}/release/bundle/dmg"
DMG_PATH="${DMG_DIR}/memorph_${VERSION}_${MARKER}.dmg"
STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/memorph-dmg.XXXXXX")"
trap 'rm -rf "${STAGING_DIR}"' EXIT

mkdir -p "${DMG_DIR}"
cp -R "${APP_PATH}" "${STAGING_DIR}/memorph.app"
ln -s /Applications "${STAGING_DIR}/Applications"

echo "[run] Creating DMG without Finder automation"
hdiutil create \
  -volname "memorph" \
  -srcfolder "${STAGING_DIR}" \
  -ov \
  -format UDZO \
  "${DMG_PATH}"

if [ ! -f "${DMG_PATH}" ]; then
  echo "[error] DMG was not created: ${DMG_PATH}"
  exit 1
fi

echo "[ok] Wrote ${DMG_PATH}"
