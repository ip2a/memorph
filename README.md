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

> 在不同 AI 编程 Agent 之间无缝迁移会话

 

memorph 是一个会话记忆管理工具，在不同AI 编程 Agent 之间导出和切换宝贵的上下文。

---


### 快速上手

#### 安装方式

```bash
curl -fsSL https://raw.githubusercontent.com/ip2a/memorph/main/install.sh | bash
```

或通过包管理器安装：

| 包管理器 | 安装命令 | 直接运行 |
|---------|---------|---------|
| npm | `npm install -g memorph` | `memorph <命令>` |
| uv | `uv tool install memorph` | `memorph <命令>` |
| pip | `pip install memorph` | `memorph <命令>` |

完成安装之后，可以使用`memo`或者完整的命令`memorph`


---


### Web

可以直接运行：

```bash
npx memorph web
```


在这里可以看到一个简洁的web操作页面

   ![启动界面](assets/zh/web-start.png)

在列表中选择要迁移的会话

   ![选择会话](assets/zh/web-select.png)

点击迁移，验证目标工具中是否加载成功

   ![迁移完成](assets/zh/web-switch.png)

### TUI

```bash
npx memorph
```
会自动以当前的路径为工作空间展开，确保在几步的简洁操作内就可以迁移会话

   ![TUI模式](assets/zh/tui.png)



### CLI

CLI模式更多的是结合`SKILLS`之后利用agent帮你管理会话的迁移与管理

#### 查看当前工作区的可用会话-list

```bash
memorph list [OPTIONS]
```

| 参数 | 说明 |
|------|------|
| `--all` | 显示所有工作区的会话 |
| `--claude` | 仅显示 Claude Code 会话 |
| `--codex` | 仅显示 Codex 会话 |
| `--opencode` | 仅显示 OpenCode 会话 |

##### 示例
```bash
$ memorph list
```
   ![列出会话](assets/zh/show-list.png)


#### 将指定会话迁移-switch

```bash
memorph switch --<from>2<to> [OPTIONS]
```

| 参数 | 说明 |
|------|------|
| `--claude2codex` | Claude → Codex |
| `--codex2claude` | Codex → Claude |
| `--claude2opencode` | Claude → OpenCode |
| `--opencode2claude` | OpenCode → Claude |
| `--codex2opencode` | Codex → OpenCode |
| `--opencode2codex` | OpenCode → Codex |
| `-s, --session-id <ID>` | 源会话 ID，**省略则取当前工作区最近会话** |
| `-d, --to-dir <DIR>` | 目标目录，**默认：当前目录** |


##### 示例
```bash
$ memorph switch --claude2codex --session-id abc-123-session-id
```

   ![迁移会话](assets/zh/show-list.png)

---

#### CLI 表格

| 命令 | 说明 |
|------|------|
| [`list`](#查看当前工作区的可用会话-list) | 列出会话（默认仅当前工作区） |
| [`export`](#export) | 导出会话为 `.json` / `.md` / `.html` 文件 |
| [`import`](#import) | 将 `.json` / `.md` / `.html` 文件或已有会话导入目标工具 |
| [`remove`](#remove--rename--find) | 删除指定会话 |
| [`rename`](#remove--rename--find) | 重命名指定会话 |
| [`switch`](#将指定会话迁移-switch) | 跨 Provider 迁移会话 |
| [`find`](#remove--rename--find) | 按目录、标题或 ID 搜索会话 |


---

#### 导入与导出

##### 导出

```bash
memorph export <PROVIDER> <SESSION_ID> [OPTIONS]
```

| 参数 | 说明 |
|------|------|
| `PROVIDER` | 目标 Agent：`claude` / `codex` / `opencode` |
| `SESSION_ID` | 会话 ID |
| `-o, --output <PREFIX>` | 输出文件名前缀，**默认：`SESSION_ID`** |
| `-f, --format <FORMAT>` | `json` / `md` / `html`，**默认：`json`** |

```bash
memorph export claude abc-123-session-id
memorph export claude abc-123-session-id -f md
```

##### 导入

```bash
memorph import <PROVIDER> <FILE_OR_ID> [OPTIONS]
```

| 参数 | 说明 |
|------|------|
| `PROVIDER` | 目标 Agent：`claude` / `codex` / `opencode` |
| `FILE_OR_ID` | `.json`/`.md`/`.html` 文件路径，或已有会话 ID |
| `-d, --to-dir <DIR>` | 目标项目目录，**默认：当前目录** |

```bash
memorph import codex ./my-session.md
memorph import claude ./backup.json --to-dir ~/projects/my-app
memorph import claude xyz-456-session-id
```

---

#### 删除与重命名

| 命令 | 语法 | 说明 |
|------|------|------|
| `remove` | `memorph remove <PROVIDER> <SESSION_ID>` | 删除会话 |
| `rename` | `memorph rename <PROVIDER> <SESSION_ID> <NEW_TITLE>` | 重命名会话 |

```bash
# 删除
memorph remove claude abc-123-session-id

# 重命名
memorph rename claude abc-123-session-id "修复登录 bug"
```

---

#### 查询

```bash
memorph find [OPTIONS]
```

| 参数 | 说明 |
|------|------|
| `-d, --dir <DIR>` | 按项目目录路径模糊搜索 |
| `-s, --session <PATTERN>` | 按会话 ID 或标题模糊匹配 |
| `-p, --provider <PROVIDER>` | 限制 Provider（可多次使用） |

**查询约束：** 至少需提供 `--dir`、`--session`、`--provider` 中的一个。

```bash
# 按目录搜索
memorph find --dir ~/projects/my-app

# 按标题或 ID 搜索
memorph find --session "登录 bug"

# 只在 Claude 和 Codex 中搜索
memorph find --session "refactor" -p claude -p codex
```

---

### 桌面端

目前已经构建了mac端的dmg版本，稳定后会逐步支持全平台的桌面端

   ![桌面端](assets/zh/dmg-app.png)


---

## Star History

<div align="center">

[![Star History Chart](https://api.star-history.com/svg?repos=ip2a/memorph&type=Date)](https://star-history.com/#ip2a/memorph&Date)

</div>

---

