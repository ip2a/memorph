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

#### View Available Sessions in Current Workspace — list

```bash
memorph list [OPTIONS]
```

| Parameter | Description |
|------|------|
| `--all` | Show sessions from all workspaces |
| `--claude` | Show only Claude Code sessions |
| `--codex` | Show only Codex sessions |
| `--opencode` | Show only OpenCode sessions |

##### Example
```bash
$ memorph list
```
   ![List Sessions](assets/en/show-list.png)


#### Migrate a Specific Session — switch

```bash
memorph switch --<from>2<to> [OPTIONS]
```

| Parameter | Description |
|------|------|
| `--claude2codex` | Claude → Codex |
| `--codex2claude` | Codex → Claude |
| `--claude2opencode` | Claude → OpenCode |
| `--opencode2claude` | OpenCode → Claude |
| `--codex2opencode` | Codex → OpenCode |
| `--opencode2codex` | OpenCode → Codex |
| `-s, --session-id <ID>` | Source session ID; **omitting this uses the most recent session in the current workspace** |
| `-d, --to-dir <DIR>` | Target directory; **default: current directory** |


##### Example
```bash
$ memorph switch --claude2codex --session-id abc-123-session-id
```

   ![Migrate Session](assets/en/show-list.png)

---

#### CLI Commands

| Command | Description |
|------|------|
| [`list`](#view-available-sessions-in-current-workspace--list) | List sessions (current workspace only by default) |
| [`export`](#export) | Export session as `.json` / `.md` / `.html` file |
| [`import`](#import) | Import `.json` / `.md` / `.html` file or existing session into target tool |
| [`remove`](#remove--rename--find) | Remove a specific session |
| [`rename`](#remove--rename--find) | Rename a specific session |
| [`switch`](#migrate-a-specific-session--switch) | Migrate session across providers |
| [`find`](#remove--rename--find) | Search sessions by directory, title, or ID |


---

#### Import and Export

##### Export

```bash
memorph export <PROVIDER> <SESSION_ID> [OPTIONS]
```

| Parameter | Description |
|------|------|
| `PROVIDER` | Target agent: `claude` / `codex` / `opencode` |
| `SESSION_ID` | Session ID |
| `-o, --output <PREFIX>` | Output filename prefix; **default: `SESSION_ID`** |
| `-f, --format <FORMAT>` | `json` / `md` / `html`; **default: `json`** |

```bash
memorph export claude abc-123-session-id
memorph export claude abc-123-session-id -f md
```

##### Import

```bash
memorph import <PROVIDER> <FILE_OR_ID> [OPTIONS]
```

| Parameter | Description |
|------|------|
| `PROVIDER` | Target agent: `claude` / `codex` / `opencode` |
| `FILE_OR_ID` | Path to `.json`/`.md`/`.html` file, or an existing session ID |
| `-d, --to-dir <DIR>` | Target project directory; **default: current directory** |

```bash
memorph import codex ./my-session.md
memorph import claude ./backup.json --to-dir ~/projects/my-app
memorph import claude xyz-456-session-id
```

---

#### Remove and Rename

| Command | Syntax | Description |
|------|------|------|
| `remove` | `memorph remove <PROVIDER> <SESSION_ID>` | Remove a session |
| `rename` | `memorph rename <PROVIDER> <SESSION_ID> <NEW_TITLE>` | Rename a session |

```bash
# Remove
memorph remove claude abc-123-session-id

# Rename
memorph rename claude abc-123-session-id "Fix login bug"
```

---

#### Search

```bash
memorph find [OPTIONS]
```

| Parameter | Description |
|------|------|
| `-d, --dir <DIR>` | Fuzzy search by project directory path |
| `-s, --session <PATTERN>` | Fuzzy match by session ID or title |
| `-p, --provider <PROVIDER>` | Restrict provider (can be used multiple times) |

**Search constraint:** At least one of `--dir`, `--session`, or `--provider` must be provided.

```bash
# Search by directory
memorph find --dir ~/projects/my-app

# Search by title or ID
memorph find --session "login bug"

# Search only in Claude and Codex
memorph find --session "refactor" -p claude -p codex
```

---


---

## Star History

<div align="center">

[![Star History Chart](https://api.star-history.com/svg?repos=ip2a/memorph&type=Date)](https://star-history.com/#ip2a/memorph&Date)

</div>

---
