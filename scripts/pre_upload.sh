#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MEMORPH_MANIFEST="$ROOT_DIR/rust/crates/memorph/Cargo.toml"
TAURI_MANIFEST="$ROOT_DIR/desktop/tauri/Cargo.toml"
WEB_DIR="$ROOT_DIR/apps/web"
NPM_VERSION="${NPM_VERSION:-11.18.0}"

for bin_dir in /usr/local/bin /opt/homebrew/bin; do
  if [ -d "$bin_dir" ] && [[ ":$PATH:" != *":$bin_dir:"* ]]; then
    PATH="$bin_dir:$PATH"
  fi
done

require_command() {
  local command_name="$1"
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "[error] 找不到命令: $command_name" >&2
    exit 1
  fi
}

run_step() {
  local title="$1"
  shift
  echo ""
  echo "[step] $title"
  "$@"
}

npm_ci() {
  npx -y "npm@$NPM_VERSION" ci "$@"
}

read_current_version() {
  python3 - "$MEMORPH_MANIFEST" <<'PY'
import re
import sys

path = sys.argv[1]
text = open(path, encoding="utf-8").read()
match = re.search(r'^version = "([^"]+)"', text, re.MULTILINE)
if not match:
    print("[error] 无法从 Cargo.toml 读取版本号", file=sys.stderr)
    sys.exit(1)
print(match.group(1))
PY
}

bump_patch_version() {
  local version="$1"
  python3 - "$version" <<'PY'
import sys

version = sys.argv[1]
parts = version.split(".")
if len(parts) != 3 or not all(part.isdigit() for part in parts):
    print(f"[error] 版本号格式不正确: {version}", file=sys.stderr)
    sys.exit(1)
print(f"{parts[0]}.{parts[1]}.{int(parts[2]) + 1}")
PY
}

update_cargo_version() {
  local new_version="$1"
  python3 - "$MEMORPH_MANIFEST" "$new_version" <<'PY'
import re
import sys

path, new_version = sys.argv[1], sys.argv[2]
with open(path, encoding="utf-8") as f:
    text = f.read()
if not re.search(r'^version = "[^"]+"', text, flags=re.MULTILINE):
    raise SystemExit("[error] 未找到 Cargo.toml 版本行")
new_text = re.sub(
    r'^version = "[^"]+"',
    f'version = "{new_version}"',
    text,
    count=1,
    flags=re.MULTILINE,
)
if new_text == text:
    print(f"[info] Cargo.toml 版本已是 {new_version}，无需修改")
else:
    with open(path, "w", encoding="utf-8") as f:
        f.write(new_text)
    print(f"[ok] 更新 Cargo.toml 版本为 {new_version}")
PY
}

choose_version() {
  local current_version next_version input_version
  current_version="$(read_current_version)"
  next_version="$(bump_patch_version "$current_version")"

  echo ""
  echo "当前版本: $current_version"
  read -r -p "请输入新版本号（直接回车使用 ${next_version}）: " input_version
  input_version="${input_version:-$next_version}"

  if ! [[ "$input_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "[error] 版本号格式不正确: $input_version（应为 x.y.z）" >&2
    exit 1
  fi

  SELECTED_VERSION="$input_version"
}

print_next_git_steps() {
  local version="$1"
  echo ""
  echo "[ok] 上传前本地准备完成。"
  echo ""
  echo "[info] 当前 git 状态:"
  git status --short
  echo ""
  echo "[info] 变更统计:"
  git diff --stat
  echo ""
  echo "接下来只需要你手动执行 git 操作，例如:"
  echo "  git add -A"
  echo "  git commit -m \"Release v$version\""
  echo "  git tag v$version"
  echo "  git push && git push origin v$version"
  echo ""
  echo "注意：tag 必须打在包含这些检查结果的最新 commit 上；不要重跑旧 tag 的 workflow。"
}

main() {
  require_command python3
  require_command npm
  require_command npx
  require_command cargo
  require_command git

  local new_version
  SELECTED_VERSION=""
  choose_version
  new_version="$SELECTED_VERSION"

  run_step "更新 Cargo.toml 版本" update_cargo_version "$new_version"
  run_step "同步所有包版本和锁文件" python3 "$ROOT_DIR/scripts/sync_version.py"
  run_step "检查版本同步结果" python3 "$ROOT_DIR/scripts/sync_version.py" --check
  run_step "运行 release_preflight 检查" python3 "$ROOT_DIR/scripts/release_preflight.py"
  run_step "验证 GitHub Linux npm ci 依赖树（npm ${NPM_VERSION}）" npm_ci --prefix "$WEB_DIR" --include=optional --os=linux --cpu=x64 --dry-run
  run_step "前端本机 clean install（npm ${NPM_VERSION}）" npm_ci --prefix "$WEB_DIR"
  run_step "前端 lint" npm --prefix "$WEB_DIR" run lint
  run_step "构建并同步前端资源到 crate" "$ROOT_DIR/scripts/build_web_assets.sh"
  run_step "运行 web UI 不变量检查" python3 "$ROOT_DIR/scripts/test_web_ui_invariants.py"
  run_step "检查 Rust workspace" cargo check --locked --manifest-path "$ROOT_DIR/rust/Cargo.toml"
  run_step "检查 Tauri desktop" cargo check --locked --manifest-path "$TAURI_MANIFEST"

  print_next_git_steps "$new_version"
}

main "$@"
