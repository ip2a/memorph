use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "memorph")]
#[command(
    about = "Convert, import, and export AI coding sessions. Run without a command to open the interactive TUI."
)]
#[command(disable_version_flag = true)]
#[command(
    help_template = "{about-with-newline}\nUsage: {usage}\n\nSessions:\n  list      List sessions and provider capabilities\n  switch    Switch a session between providers (alias: migrate)\n  export    Export a session to file(s)\n  import    Import a session into a provider\n  remove    Remove a session\n  rename    Rename a session\n\nInterfaces:\n  web       Start the web UI server\n  api       Start the API server only\n  tui       Start the interactive TUI\n\nMaintenance:\n  doctor    Run read-only environment diagnostics\n  update    Update memorph using detected install source\n\nOptions:\n{options}\n\n{after-help}"
)]
#[command(
    after_help = "Examples:\n  memorph list --all --title bug\n  memorph --json list --since 7d\n  memorph switch claude codex --session-id SESSION_ID"
)]
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
        after_help = "Examples:\n  memorph list --all --title bug --since 7d --min-bytes 1K\n  memorph --json list --text needle | jq .\n  memorph list --providers --provider codex"
    )]
    List {
        /// Show all sessions across all workspaces
        #[arg(long)]
        all: bool,
        /// Filter to a provider (repeatable)
        #[arg(short, long, value_name = "PROVIDER")]
        provider: Vec<String>,
        /// Sort sessions
        #[arg(long, value_enum, default_value_t = ListSort::Recent)]
        sort: ListSort,
        /// Maximum sessions to return
        #[arg(long)]
        limit: Option<usize>,
        /// Number of sessions to skip
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Match project directory or source locator
        #[arg(short, long, value_name = "DIR")]
        dir: Option<String>,
        /// Match provider/canonical session ID or title
        #[arg(short = 's', long, value_name = "PATTERN")]
        session: Option<String>,
        /// Match session title
        #[arg(long, value_name = "PATTERN")]
        title: Option<String>,
        /// Match imported message body text
        #[arg(long, value_name = "PATTERN")]
        text: Option<String>,
        /// Only sessions active after TIME: date, RFC3339, 7d, 24h, 30m
        #[arg(long, value_name = "TIME")]
        since: Option<String>,
        /// Only sessions active before TIME
        #[arg(long, value_name = "TIME")]
        before: Option<String>,
        /// Minimum source size: bytes, K, M, or G
        #[arg(long, value_name = "BYTES")]
        min_bytes: Option<String>,
        /// Maximum source size
        #[arg(long, value_name = "BYTES")]
        max_bytes: Option<String>,
        /// Show provider capabilities instead of sessions
        #[arg(long)]
        providers: bool,
    },
    /// Switch a session from one provider to another
    #[command(
        alias = "migrate",
        after_help = "Examples:\n  memorph switch claude codex --session-id SESSION_ID\n  memorph migrate claude codex --session-id SESSION_ID"
    )]
    Switch {
        #[arg(value_name = "FROM")]
        from: String,
        #[arg(value_name = "TO")]
        to: String,
        #[arg(short, long, value_name = "ID")]
        session_id: Option<String>,
        #[arg(short, long, value_name = "DIR")]
        to_dir: Option<String>,
    },
    /// Export a session to file(s)
    Export {
        #[arg(value_name = "PROVIDER")]
        provider: String,
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
        #[arg(short, long, value_name = "PREFIX")]
        output: Option<String>,
        #[arg(short, long, value_name = "FORMAT", default_value = "json")]
        format: String,
    },
    /// Import a session into a target provider directory
    Import {
        #[arg(value_name = "PROVIDER")]
        provider: String,
        #[arg(value_name = "FILE_OR_ID")]
        file_or_id: String,
        #[arg(short, long, value_name = "DIR")]
        to_dir: Option<String>,
    },
    /// Remove a session
    Remove {
        #[arg(value_name = "PROVIDER")]
        provider: String,
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
    },
    /// Rename a session
    Rename {
        #[arg(value_name = "PROVIDER")]
        provider: String,
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
        #[arg(value_name = "NEW_TITLE")]
        new_title: String,
    },
    /// Start the web UI server
    Web {
        #[arg(short, long)]
        port: Option<u16>,
        #[arg(long)]
        no_open: bool,
    },
    /// Start the API server only
    Api {
        #[arg(short, long)]
        port: Option<u16>,
    },
    /// Start the interactive TUI
    Tui,
    /// Run read-only environment diagnostics
    #[command(after_help = "Examples:\n  memorph doctor\n  memorph --json doctor | jq .")]
    Doctor,
    /// Update memorph using detected install source
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
    fn top_level_help_is_grouped_and_hides_hook_bridge() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("Sessions:"));
        assert!(help.contains("Interfaces:"));
        assert!(help.contains("Maintenance:"));
        assert!(help.contains("Run without a command to open the interactive TUI."));
        assert!(!help.contains("__hook-bridge"));
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
