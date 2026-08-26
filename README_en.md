<div align="center">

<table align="center">
  <tr>
    <td>
<pre>
███   ███ ███████ ███   ███  █████  ███████ ███████ ██   ██
████ ████ ██      ████ ████ ██   ██ ██   ██ ██   ██ ██   ██
██ ███ ██ ███████ ██ ███ ██ ██   ██ ██████  ███████ ███████
██  █  ██ ██      ██  █  ██ ██   ██ ██   ██ ██      ██   ██
██     ██ ███████ ██     ██  █████  ██   ██ ██      ██   ██
</pre>
    </td>
  </tr>
</table>

---

![GitHub stars](https://img.shields.io/github/stars/ip2a/memorph) ![GitHub forks](https://img.shields.io/github/forks/ip2a/memorph) ![GitHub license](https://img.shields.io/github/license/ip2a/memorph) ![npm version](https://img.shields.io/npm/v/memorph)

[English](README_en.md) | [简体中文](README_zh.md)


</div>

> Seamlessly migrate sessions between different AI coding agents

 

memorph is a session memory management tool that exports and switches valuable context between different AI coding agents.

---


### Quick Start

```bash
npx memorph web
```

Or install via a package manager:

| Package Manager | Install Command | Run Directly |
|---------|---------|---------|
| npm | `npm install -g memorph` | `memorph <command>` |
| uv | `uv tool install memorph` | `memorph <command>` |
| pip | `pip install memorph` | `memorph <command>` |

After installation, use `memorph web` to launch the Web UI, or run `memorph <command>` directly to use the CLI.

---


### Web

```bash
memorph web
```


Here you'll see a clean web interface.

   ![Startup Interface](assets/en/web-start.png)

Select the session you want to migrate from the list.

   ![Select Session](assets/en/web-select.png)

Click migrate and verify it loads successfully in the target tool.

   ![Migration Complete](assets/en/web-switch.png)

### CLI

Use CLI for scripts, agents, and manual operations. Running without a command opens the TUI. Every command supports global `--json`.

#### List and filter sessions: list

```bash
memorph list [OPTIONS]
```

| Option | Description |
|------|------|
| `--all` | Show sessions from all workspaces |
| `-p, --provider <PROVIDER>` | Restrict provider; repeatable |
| `--sort <recent/title>` | Sort order |
| `--limit <N>` / `--offset <N>` | Pagination |
| `-d, --dir <DIR>` | Match project directory or source path |
| `-s, --session <PATTERN>` | Match session ID or title |
| `--title <PATTERN>` | Match session title |
| `--text <PATTERN>` | Search message body text |
| `--since <TIME>` / `--before <TIME>` | Time filter: date, RFC3339, `7d`, `24h`, `30m` |
| `--min-bytes <BYTES>` / `--max-bytes <BYTES>` | Size filter: bytes, `K`, `M`, `G` |
| `--providers` | Show provider capabilities |
| `--json` | Output JSON; may appear before or after the command |

```bash
memorph list
memorph list --all --title "login" --since 7d
memorph --json list --text "error" | jq .
memorph list --providers --provider codex
```

   ![List Sessions](assets/en/show-list.png)

#### Switch sessions: switch

```bash
memorph switch <FROM> <TO> [OPTIONS]
memorph migrate <FROM> <TO> [OPTIONS]  # alias for switch
```

| Option | Description |
|------|------|
| `FROM` / `TO` | Source provider / target provider |
| `-s, --session-id <ID>` | Source session ID; omitted uses the latest session in the current workspace |
| `-t, --to-dir <DIR>` | Target project directory |

```bash
memorph switch claude codex --session-id abc-123-session-id
memorph migrate claude codex --session-id abc-123-session-id
```

   ![Migrate Session](assets/en/show-list.png)

#### CLI commands

| Command | Description |
|------|------|
| `list` | List and filter sessions, or show provider capabilities |
| `switch` / `migrate` | Migrate a session across providers |
| `export` | Export a session |
| `import` | Import a session |
| `remove` | Remove a session |
| `rename` | Rename a session |
| `web` / `api` / `tui` | Start Web, API, or TUI |
| `doctor` | Run read-only environment diagnostics |
| `update` | Update memorph using detected install source |

#### Import and export

```bash
memorph export <PROVIDER> <SESSION_ID> [-o PREFIX] [-f FORMAT]
memorph import <PROVIDER> <FILE_OR_ID> [-t DIR]
```

```bash
memorph export claude abc-123-session-id -f md
memorph import claude ./backup.json --to-dir ~/projects/my-app
```

#### Remove and rename

```bash
memorph remove <PROVIDER> <SESSION_ID>
memorph rename <PROVIDER> <SESSION_ID> <NEW_TITLE>
```

```bash
memorph remove claude abc-123-session-id
memorph rename claude abc-123-session-id "Fix login bug"
```

### Provider Support Matrix

memorph supports session management for 23 AI coding agents. Capabilities differ per provider:

| Provider | Import | Export | Manage/Resume |
|---|---:|---:|---:|
| Claude | ✅ | ✅ | ✅ |
| Codex | ✅ | ✅ | ✅ |
| OpenCode | ✅ | ✅ | ✅ |
| DeepSeek | ✅ | ✅ | ✅ |
| Kimi | ✅ | ✅ | ✅ |
| Cursor | ✅ | ✅ | ✅ |
| Gemini CLI | ✅ | ❌ | ✅ |
| Kiro | ✅ | ❌ | ✅ |
| Hermes | ✅ | ❌ | ✅ |
| Cline | ✅ | ❌ | ❌ |
| Copilot | ✅ | ❌ | ❌ |
| Droid | ✅ | ❌ | ❌ |
| CodeBuddy | ✅ | ❌ | ❌ |
| Qoder | ✅ | ❌ | ❌ |
| Trae | ✅ | ❌ | ❌ |
| WorkBuddy | ✅ | ❌ | ❌ |
| Pi | ✅ | ❌ | ❌ |
| Antigravity | ✅ | ❌ | ❌ |
| OpenClaw | ✅ | ❌ | ❌ |
| Augment | ✅ | ❌ | ❌ |
| Windsurf | ✅ | ❌ | ❌ |
| Amazon Q | ✅ | ❌ | ❌ |
| Qwen | ✅ | ❌ | ❌ |

> **Export**: write OASF canonical Session back as provider-native format.  
> **Manage/Resume**: supports at least one of delete / rename / resume.  
> Import uses a unified OASF Session model; export fidelity depends on the target provider's native format capabilities.

### OASF Compatibility

memorph is built on [OASF (Open Agent Session Format)](https://crates.io/crates/oasf) v2 (crate `oasf` 0.2.0).

- All session files (`.morph`, `.json`, `.md`, `.html`) carry schema name/version in the meta line; validated on import, rejecting incompatible versions
- Import: 23 providers' native formats are unified into OASF canonical Session
- Export: 6 providers support writing canonical Session back to native format (Claude, Codex, OpenCode, DeepSeek, Kimi, Cursor)
- Fidelity model: each Provider × Block type declares `Preserved` / `Normalized` / `Downgraded` / `Dropped` / `Unsupported`, with loss reports generated at export time via `export_report`

---

## Star History

<div align="center">

[![Star History Chart](https://api.star-history.com/svg?repos=ip2a/memorph&type=Date)](https://star-history.com/#ip2a/memorph&Date)

</div>

---
