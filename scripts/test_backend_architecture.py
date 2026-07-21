#!/usr/bin/env python3
"""Repository-level acceptance checks for the backend architecture migration."""
from __future__ import annotations
import json, re, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MATRIX = ROOT / 'docs/backend-architecture-refactor-capability-matrix.md'
SRC = ROOT / 'rust/crates/memorph/src'
CONTRACTS = ROOT / 'docs/contracts'
FAIL = []

def check(ok: bool, message: str):
    if not ok: FAIL.append(message)

def load(name):
    p=CONTRACTS/name
    check(p.is_file(), f'缺少 contract snapshot: {p}')
    if not p.is_file(): return {}
    try: return json.loads(p.read_text())
    except Exception as e: FAIL.append(f'contract snapshot 无效 JSON: {p}: {e}'); return {}

def main():
    text=MATRIX.read_text() if MATRIX.exists() else ''
    check(MATRIX.exists(), '缺少能力矩阵')
    # Only table rows are state-bearing; explanatory prose may mention forbidden words.
    rows=[l for l in text.splitlines() if l.startswith('|') and l.count('|') >= 6 and not l.startswith('|---')]
    check(rows, '能力矩阵没有可解析的能力行')
    allowed={'baseline','in_progress','blocked','done'}
    for row in rows:
        cells=[x.strip() for x in row.strip('|').split('|')]
        if cells and cells[0] in ('能力域','---'): continue
        check(len(cells)>=6, f'能力矩阵列数不足: {row}')
        if len(cells)>=6:
            check(cells[2] not in ('','unknown','pending'), f'能力缺少真实迁移后入口: {cells[0]}')
            check(cells[5] in allowed, f'能力状态非法: {cells[0]}={cells[5]}')
            check(cells[4] not in ('','—','-'), f'能力缺少关键测试: {cells[0]}')
    router=(SRC/'api/router.rs').read_text()
    routes=load('api-routes.json').get('routes',[])
    actual=set(re.findall(r'\.route\(\s*"([^"]+)"',router))
    snap=set(x.get('path') for x in routes)
    check(actual==snap, f'API route snapshot 不匹配: source={len(actual)} snapshot={len(snap)}')
    check(all(x.get('methods') for x in routes), 'API route snapshot 存在无 HTTP method 的条目')
    cli=(SRC/'cli.rs').read_text()
    cli_snap=load('cli-surface.json').get('commands',[])
    actual_cli=set(re.findall(r'^    ([A-Z][A-Za-z0-9_]*)\s*\{', cli[cli.index('pub enum Commands'):cli.index('pub enum SessionCommands')], re.M))
    check(actual_cli==set(cli_snap), 'CLI command snapshot 不匹配')
    providers=load('provider-registry.json').get('providers',[])
    actual_providers={p.name for p in (SRC/'providers').iterdir() if p.is_dir() and (p/'mod.rs').exists()}
    check(actual_providers==set(providers), 'Provider registry snapshot 不匹配')
    storage=load('storage-contracts.json').get('required_modules',[])
    check(all((SRC/'storage'/p).is_file() for p in storage), 'storage contract 引用不存在的模块')
    # Architectural dependency rules. Tests and documentation are excluded.
    for path in (SRC/'domain').rglob('*.rs') if (SRC/'domain').exists() else []:
        s=path.read_text(); check(not re.search(r'\b(axum|clap|ratatui|rusqlite|sqlx)\b',s), f'domain 依赖基础设施: {path}')
    for path in (SRC/'application').rglob('*.rs') if (SRC/'application').exists() else []:
        s=path.read_text(); check(not re.search(r'\b(axum|clap|ratatui)\b',s), f'application 依赖接口框架: {path}')
    for path in (SRC/'providers').rglob('*.rs'):
        s=path.read_text(); check(not re.search(r'use crate::(api|interfaces)\b',s), f'provider 依赖 API/interface: {path}')
    core=(SRC/'core.rs').read_text()
    check(len(core.splitlines()) < 3500, f'core.rs 仍超过 3500 行 ({len(core.splitlines())})，不得视为收口')
    # Never stage known user worktree changes as part of migration.
    out=subprocess.run(['git','diff','--cached','--name-only'],cwd=ROOT,text=True,capture_output=True,check=False).stdout.splitlines()
    protected={'apps/web/src/features/hooks/hooks-page.tsx','apps/web/src/features/hooks/queries.ts','apps/web/src/features/workspaces/workspace-switch-dialog.tsx','apps/web/src/lib/api.ts','apps/web/src/lib/query-keys.ts','apps/web/src/lib/types.ts','rust/crates/memorph/src/cli.rs','rust/crates/memorph/src/hooks/mod.rs','rust/crates/memorph/src/hooks/server.rs','rust/crates/memorph/src/tui/app.rs','apps/web/src/lib/api.test.ts','rust/crates/memorph/src/hooks/discovery.rs'}
    check(not protected.intersection(out), '用户原有改动被 staged: '+', '.join(sorted(protected.intersection(out))))
    if FAIL:
        print('BACKEND ARCHITECTURE ACCEPTANCE: FAIL')
        print('\n'.join(f'- {x}' for x in FAIL)); return 1
    print('BACKEND ARCHITECTURE ACCEPTANCE: PASS')
    return 0
if __name__=='__main__': sys.exit(main())
