# Changelog

## 0.1.32 — 2026-08-11

### Session Conversion & Fidelity

- **Schema validation**: `read_session_export_file` now validates OASF schema
  name/version on import, rejecting files written by incompatible OASF
  revisions (commit 98de8a5).

- **Fidelity guards**: two parameterized tests verify all 23 providers'
  declared fidelity is consistent with their capabilities and that
  `export_report` accurately reflects declared export fidelity (commit 2e5f394).

### Prior Fixes (commits e646d40..3b33d7f)

- Codex/OpenCode: emit native tool calls (`function_call`/`function_call_output`)
  on export instead of downgrading to text.
- OpenCode: export `Block::Patch` as native patch part.
- All providers: unified `EventKind` priority — `ToolResult→Observation`
  wins over `ToolCall→Action`.
- Provider fidelity declarations corrected across 23 providers
  (import/export fidelity now matches actual code behavior).
- `export_block_fidelity` fixed for `Block::Compressed` (was reading
  `fidelity.text` instead of `fidelity.compressed`).
- Augment: completed `import_fidelity` declaration.

### OASF Compatibility

- OASF schema: `oasf` v2 (crate `oasf` 0.2.0).
- Session files (`.morph`, `.json`, `.md`, `.html`) carry schema name/version
  in the meta line; mismatched schemas are rejected on import.
- Provider extensions use `{provider}_{field}` naming convention in
  `Session.extensions`.
