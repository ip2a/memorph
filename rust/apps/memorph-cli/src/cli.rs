use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "memorph")]
#[command(
    about = "Convert, import, and export AI coding sessions.",
    long_about = "Convert, import, and export AI coding sessions. Run without a command to open the interactive TUI.

Agent workflow:
  1. Discover sessions: memorph --json list --all
  2. Narrow results: add --provider, --title, --text, --since, or --dir
  3. Act on a result: pass its provider_id and session_id to a command
  4. Diagnose setup: memorph --json doctor

Defaults:
  list shows sessions in the current workspace; --all includes every workspace.
  Filters combine with AND. Repeat --provider to select several providers.
  Use `memorph help <command>` or `memorph <command> --help` for command details."
)]
#[command(disable_version_flag = true)]
#[command(help_template = "{about-with-newline}Usage: {usage}

Sessions:
  list      List and filter projected sessions
  switch    Copy a session between providers (alias: migrate)
  export    Write a session to portable file(s)
  import    Write a portable session into a provider
  remove    Delete a provider session
  rename    Change a session title

Interfaces:
  web       Start the Web UI server
  api       Start the API server only
  tui       Start the interactive TUI

Maintenance:
  doctor    Inspect database and provider health (read-only)
  update    Update memorph using detected install source

Global options:
{options}
{after-help}")]
#[command(after_help = "Agent contract:
  --json may appear before or after the command.
  In JSON mode, success writes valid JSON to stdout; schema is command-specific.
  Errors write {\"ok\":false,\"error\":\"...\"} to stdout and exit non-zero.

Identifiers:
  list returns provider groups. Use provider_id from a group and session_id from
  one of its sessions together for export, switch, rename, and remove.

Common recipes:
  memorph --json list --all --limit 20
  memorph --json list --all --provider codex --text \"needle\" | jq .
  memorph --json export codex SESSION_ID --format json --output /tmp/session
  memorph --json switch claude codex --session-id SESSION_ID
  memorph --json doctor | jq .")]
pub struct Cli {
    /// Print version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::SetTrue)]
    pub version: bool,
    /// Output structured JSON; errors use JSON too
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List sessions and provider capabilities
    #[command(
        long_about = "List projected sessions, grouped by provider.

By default, only sessions belonging to the current workspace are shown. Use --all
for every workspace. Filters combine with AND. In JSON output, each group contains
provider_id and each session contains session_id; pass both identifiers to later
commands. Use --text for imported message-body search; it may be slower than metadata
filters. Use --providers to inspect provider capabilities before writing sessions.",
        after_help = "Examples:
  memorph list --all --title bug --since 7d --min-bytes 1K
  memorph --json list --all --provider codex --text \"needle\" | jq .
  memorph list --providers --provider codex"
    )]
    List {
        /// Include sessions from every workspace; default is the current workspace
        #[arg(long)]
        all: bool,
        /// Restrict to a provider; repeat for multiple providers
        #[arg(short, long, value_name = "PROVIDER")]
        provider: Vec<String>,
        /// Sort results by most recent activity or title
        #[arg(long, value_enum, default_value_t = ListSort::Recent)]
        sort: ListSort,
        /// Return at most this many sessions per provider group
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
        /// Skip this many sessions in each provider group
        #[arg(long, default_value_t = 0, value_name = "N")]
        offset: usize,
        /// Match project directory or source locator
        #[arg(short, long, value_name = "DIR")]
        dir: Option<String>,
        /// Match provider or canonical session ID, or title
        #[arg(short = 's', long, value_name = "PATTERN")]
        session: Option<String>,
        /// Match session title
        #[arg(long, value_name = "PATTERN")]
        title: Option<String>,
        /// Search imported message-body text; can be slower than metadata filters
        #[arg(long, value_name = "PATTERN")]
        text: Option<String>,
        /// Keep sessions active after TIME: date, RFC3339, 7d, 24h, or 30m
        #[arg(long, value_name = "TIME")]
        since: Option<String>,
        /// Keep sessions active before TIME
        #[arg(long, value_name = "TIME")]
        before: Option<String>,
        /// Keep sessions whose source is at least this size: bytes, K, M, or G
        #[arg(long, value_name = "BYTES")]
        min_bytes: Option<String>,
        /// Keep sessions whose source is at most this size
        #[arg(long, value_name = "BYTES")]
        max_bytes: Option<String>,
        /// Show provider capabilities instead of sessions
        #[arg(long)]
        providers: bool,
    },
    /// Copy a session from one provider to another
    #[command(
        alias = "migrate",
        long_about = "Copy a source session into a target provider.

The source session remains unchanged. If --session-id is omitted, the most recent
source session in the current workspace is selected. Use --to-dir to choose the
target workspace. JSON output includes source_session_id, target_session_id, and
an optional resume_command.",
        after_help = "Examples:
  memorph switch claude codex --session-id SESSION_ID
  memorph switch claude codex --session-id SESSION_ID --to-dir /path/to/project
  memorph migrate claude codex --session-id SESSION_ID"
    )]
    Switch {
        /// Source provider ID from list output
        #[arg(value_name = "FROM")]
        from: String,
        /// Target provider ID from list output
        #[arg(value_name = "TO")]
        to: String,
        /// Source session_id; omit to use the latest session in the current workspace
        #[arg(short, long, value_name = "SESSION_ID")]
        session_id: Option<String>,
        /// Target workspace directory
        #[arg(short, long, value_name = "DIR")]
        to_dir: Option<String>,
    },
    /// Export a session to portable file(s)
    #[command(
        long_about = "Read a session identified by PROVIDER and SESSION_ID and write portable files.

--output is a path prefix, not a directory. Without it, SESSION_ID is used as the
prefix in the current directory. Supported formats are json, md, markdown, html,
morph, and both; both writes .morph and .json files. JSON output returns the
created file paths.",
        after_help = "Examples:
  memorph export codex SESSION_ID
  memorph --json export codex SESSION_ID --format morph --output /tmp/session
  memorph export claude SESSION_ID --format both --output ./session"
    )]
    Export {
        /// Provider ID from list output
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// session_id from the provider group in list output
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
        /// Output path prefix; parent directory must exist
        #[arg(short, long, value_name = "PREFIX")]
        output: Option<String>,
        /// Format: json, md, markdown, html, morph, or both
        #[arg(short, long, value_name = "FORMAT", default_value = "json")]
        format: String,
    },
    /// Import a portable session into a target provider
    #[command(
        long_about = "Write a session into PROVIDER, which is always the target provider.

FILE_OR_ID may be a .morph, .json, .md, or .html export file. For any other value,
it is treated as an existing session_id in the same PROVIDER and imported again.
For the common cross-provider case, use switch instead. Use --to-dir to choose
target workspace. JSON output includes the new session ID and optional resume command.",
        after_help = "Examples:
  memorph import codex ./session.morph
  memorph --json import claude ./session.json --to-dir /path/to/project
  memorph import codex SESSION_ID"
    )]
    Import {
        /// Target provider ID
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// Export file path, or existing session_id in the same provider
        #[arg(value_name = "FILE_OR_ID")]
        file_or_id: String,
        /// Target workspace directory
        #[arg(short, long, value_name = "DIR")]
        to_dir: Option<String>,
    },
    /// Permanently delete a provider session
    #[command(
        long_about = "Delete the provider-native session identified by PROVIDER and SESSION_ID.

This is a destructive operation. Confirm the exact pair with `memorph --json list`
before running it. The CLI has no restore command. JSON output returns the provider
display name and session_id.",
        after_help = "Examples:
  memorph --json list --provider codex --session SESSION_ID
  memorph remove codex SESSION_ID"
    )]
    Remove {
        /// Provider ID from list output
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// session_id from the provider group in list output
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
    },
    /// Rename a session
    #[command(
        long_about = "Change the session display title. When supported, memorph also updates
the provider-native title. JSON output includes display_title, native_updated,
and any warning.",
        after_help = "Examples:
  memorph rename codex SESSION_ID \"Investigate login timeout\"
  memorph --json rename claude SESSION_ID \"Release notes\""
    )]
    Rename {
        /// Provider ID from list output
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// session_id from the provider group in list output
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
        /// New display title
        #[arg(value_name = "NEW_TITLE")]
        new_title: String,
    },
    /// Start the long-running Web UI server
    #[command(
        long_about = "Start the Web UI server on localhost.

The default port comes from configuration; --port overrides it. The server may use
a nearby fallback port if the requested port is busy. By default, open a browser;
use --no-open for headless or Agent use. In JSON mode, the first line is
{\"ok\":true,\"result\":{\"interface\":\"web\",\"url\":\"http://127.0.0.1:PORT\"}},
then the process keeps running.",
        after_help = "Examples:
  memorph web
  memorph web --port 8787 --no-open
  memorph --json web --no-open"
    )]
    Web {
        /// Listen port; default comes from configuration
        #[arg(short, long, value_name = "PORT")]
        port: Option<u16>,
        /// Do not open a browser after startup
        #[arg(long)]
        no_open: bool,
    },
    /// Start the long-running API server only
    #[command(
        long_about = "Start the API server on localhost without the Web UI.

The default port comes from configuration; --port overrides it. The server may use
a nearby fallback port if the requested port is busy. In JSON mode, the first line is
{\"ok\":true,\"result\":{\"interface\":\"api\",\"url\":\"http://127.0.0.1:PORT\"}},
then the process keeps running.",
        after_help = "Examples:
  memorph api
  memorph api --port 8788
  memorph --json api --port 8788"
    )]
    Api {
        /// Listen port; default comes from configuration
        #[arg(short, long, value_name = "PORT")]
        port: Option<u16>,
    },
    /// Start the interactive TUI
    #[command(
        long_about = "Open the interactive terminal UI. This command needs an interactive
terminal and cannot be combined with --json; JSON mode returns a structured error.",
        after_help = "Examples:
  memorph tui
  memorph"
    )]
    Tui,
    /// Run read-only environment diagnostics
    #[command(
        long_about = "Inspect memorph version, detected install source, database state, provider
scans, and orphan artifact manifests. This command does not modify sessions.
In JSON mode, output is a bare DoctorReport object rather than an ok/result envelope.",
        after_help = "Examples:
  memorph doctor
  memorph --json doctor | jq ."
    )]
    Doctor,
    /// Update memorph using the detected install source
    #[command(
        long_about = "Update memorph using the detected npm, pip, pipx, or uv tool install source.
This command has side effects. In JSON mode, output includes the detected source,
executed command, stdout, and stderr; detection or update failures exit non-zero.",
        after_help = "Examples:
  memorph update
  memorph --json update | jq ."
    )]
    Update,
    #[command(name = "__hook-bridge", hide = true)]
    /// Internal hook bridge entrypoint
    HookBridge {
        #[arg(long, value_name = "VERSION")]
        managed_version: Option<String>,
        #[arg(long, value_name = "PROVIDER")]
        provider: String,
        #[arg(long, value_name = "EVENT")]
        event: String,
        #[arg(long)]
        blocking: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ListSort {
    Recent,
    Title,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn top_level_help_is_agent_friendly_and_grouped() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("Agent workflow:"));
        assert!(help.contains("Agent contract:"));
        assert!(help.contains("provider_id"));
        assert!(help.contains("session_id"));
        assert!(help.contains("memorph help <command>"));
        assert!(help.contains("Sessions:"));
        assert!(help.contains("Interfaces:"));
        assert!(help.contains("Maintenance:"));
        assert!(!help.contains("__hook-bridge"));
    }

    #[test]
    fn every_public_command_has_examples_and_command_count_is_stable() {
        let command = Cli::command();
        let visible = command
            .get_subcommands()
            .filter(|command| !command.is_hide_set())
            .collect::<Vec<_>>();
        assert_eq!(visible.len(), 11);
        for command in visible {
            let help = command.clone().render_long_help().to_string();
            assert!(
                help.contains("Examples:"),
                "missing examples in {} help",
                command.get_name()
            );
        }
    }

    #[test]
    fn command_help_documents_runtime_semantics() {
        let command = Cli::command();
        let help = |name: &str| {
            command
                .get_subcommands()
                .find(|subcommand| subcommand.get_name() == name)
                .unwrap()
                .clone()
                .render_long_help()
                .to_string()
        };

        let list = help("list");
        assert!(list.contains("current workspace"));
        assert!(list.contains("provider_id"));
        assert!(list.contains("session_id"));
        assert!(list.contains("per provider group"));

        assert!(help("switch").contains("source session remains unchanged"));

        let export = help("export");
        for format in ["json", "md", "markdown", "html", "morph", "both"] {
            assert!(export.contains(format));
        }

        assert!(help("import").contains("always the target provider"));
        assert!(help("remove").contains("destructive operation"));
        assert!(help("tui").contains("cannot be combined with --json"));

        for name in ["web", "api"] {
            let help = help(name);
            assert!(help.contains("then the process keeps running"));
            assert!(help.contains(&format!(r#""interface":"{name}""#)));
        }
    }

    #[test]
    fn global_json_and_list_filters_parse() {
        let cli = Cli::parse_from([
            "memorph",
            "--json",
            "list",
            "--dir",
            "/tmp",
            "--title",
            "bug",
            "--text",
            "needle",
            "--since",
            "7d",
            "--max-bytes",
            "10M",
        ]);
        assert!(cli.json);
        match cli.command {
            Some(Commands::List {
                dir,
                title,
                text,
                since,
                max_bytes,
                ..
            }) => {
                assert_eq!(dir.as_deref(), Some("/tmp"));
                assert_eq!(title.as_deref(), Some("bug"));
                assert_eq!(text.as_deref(), Some("needle"));
                assert_eq!(since.as_deref(), Some("7d"));
                assert_eq!(max_bytes.as_deref(), Some("10M"));
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn json_after_subcommand_and_migrate_alias_parse() {
        let cli = Cli::parse_from(["memorph", "list", "--json"]);
        assert!(cli.json);
        let cli = Cli::parse_from(["memorph", "migrate", "claude", "codex"]);
        assert!(matches!(cli.command, Some(Commands::Switch { .. })));
    }

    #[test]
    fn removed_commands_are_not_top_level_variants() {
        for args in [
            ["memorph", "find"],
            ["memorph", "providers"],
            ["memorph", "sessions"],
            ["memorph", "backups"],
            ["memorph", "database"],
            ["memorph", "artifacts"],
            ["memorph", "codex"],
            ["memorph", "sync"],
            ["memorph", "compression"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn hook_bridge_stays_hidden_command() {
        let cli = Cli::parse_from([
            "memorph",
            "__hook-bridge",
            "--provider",
            "claude",
            "--event",
            "PreToolUse",
        ]);
        assert!(matches!(cli.command, Some(Commands::HookBridge { .. })));
    }
}
