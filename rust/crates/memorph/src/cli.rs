use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "memorph")]
#[command(about = "Convert, import, and export AI coding sessions")]
#[command(disable_version_flag = true)]
pub struct Cli {
    /// Print version
    #[arg(short = 'v', long = "version", action = clap::ArgAction::SetTrue)]
    pub version: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List sessions (default: current workspace only)
    List {
        /// Show all sessions across all workspaces
        #[arg(long)]
        all: bool,
        /// Filter to a provider (repeatable)
        #[arg(short, long, value_name = "PROVIDER")]
        provider: Vec<String>,
    },
    /// Export a session to file(s)
    Export {
        /// Source provider ID
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// Session ID
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
        /// Output filename prefix (default: SESSION_ID)
        #[arg(short, long, value_name = "PREFIX")]
        output: Option<String>,
        /// Output format: json, md, html, morph, both (default: json)
        #[arg(short, long, value_name = "FORMAT", default_value = "json")]
        format: String,
    },
    /// Import a session into a target provider directory
    Import {
        /// Target provider ID
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// Path to .json/.md/.html/.morph file, or session ID for re-export
        #[arg(value_name = "FILE_OR_ID")]
        file_or_id: String,
        /// Target project directory (default: current directory)
        #[arg(short, long, value_name = "DIR")]
        to_dir: Option<String>,
    },
    /// Remove a session
    Remove {
        /// Provider ID
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// Session ID to remove
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
    },
    /// Rename a session
    Rename {
        /// Provider ID
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// Session ID to rename
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
        /// New title
        #[arg(value_name = "NEW_TITLE")]
        new_title: String,
    },
    /// Switch a session from one provider to another (one-shot)
    Switch {
        /// Source provider ID
        #[arg(value_name = "FROM")]
        from: String,
        /// Target provider ID
        #[arg(value_name = "TO")]
        to: String,
        /// Source session ID (if omitted, uses the most recent session in current workspace)
        #[arg(short, long, value_name = "ID")]
        session_id: Option<String>,
        /// Target project directory (default: current directory)
        #[arg(short, long, value_name = "DIR")]
        to_dir: Option<String>,
    },
    /// Find sessions by directory, title, or ID pattern
    Find {
        /// Filter by project directory path (fuzzy match)
        #[arg(short, long, value_name = "DIR")]
        dir: Option<String>,
        /// Filter by session ID or title pattern (fuzzy match)
        #[arg(short, long, value_name = "PATTERN")]
        session: Option<String>,
        /// Restrict to provider (can be used multiple times)
        #[arg(short, long, value_name = "PROVIDER")]
        provider: Vec<String>,
    },
    /// Show provider capability quality and risk
    Providers {
        /// Show complete details for one provider
        #[arg(value_name = "PROVIDER")]
        provider: Option<String>,
        /// Print the capability model as JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage projected session snapshots
    Sessions {
        #[command(subcommand)]
        command: SessionCommands,
    },
    /// Query and restore registered native session backups
    Backups {
        #[command(subcommand)]
        command: BackupCommands,
    },
    /// Protect and restore the memorph management database
    Database {
        #[command(subcommand)]
        command: DatabaseCommands,
    },
    /// Inspect and explicitly clean managed artifacts
    Artifacts {
        #[command(subcommand)]
        command: ArtifactCommands,
    },
    /// Manage synchronized multi-provider sessions
    #[command(name = "sync")]
    Sync {
        #[command(subcommand)]
        command: SyncCommands,
    },
    /// Manage compressed session archives
    Compression {
        #[command(subcommand)]
        command: CompressionCommands,
    },
    /// Start the web UI server (recommended)
    Web {
        /// Port to listen on (defaults to 3737; override via server.web_port in ~/.memorph/config.json)
        #[arg(short, long)]
        port: Option<u16>,
        /// Don't auto-open browser
        #[arg(long)]
        no_open: bool,
    },
    /// Start the API server only
    Api {
        /// Port to listen on (defaults to 3223; override via server.api_port in ~/.memorph/config.json)
        #[arg(short, long)]
        port: Option<u16>,
    },
    /// Start the interactive TUI
    Tui,
    /// Run Codex-specific maintenance actions
    Codex {
        /// Sync Codex sessions for the current workspace so hidden sessions show up again
        #[arg(long)]
        sync: bool,
        /// Workspace directory to sync (default: current directory)
        #[arg(short, long, value_name = "DIR")]
        workspace: Option<String>,
        /// Explicit Codex home directory (default: ~/.codex)
        #[arg(long, value_name = "DIR")]
        codex_home: Option<String>,
        /// Number of recent sync backups to keep
        #[arg(long, default_value = "5", value_name = "N")]
        keep: usize,
    },
    /// Update memorph using the detected install source
    Update,
    #[command(name = "__hook-bridge", hide = true)]
    /// Internal hook bridge entrypoint used by installed provider hooks
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

#[derive(Subcommand)]
pub enum SessionCommands {
    /// Discover provider sessions and fully project new or changed sources into SQLite
    Bootstrap {
        /// Limit discovery and projection to one provider
        #[arg(short, long, value_name = "PROVIDER")]
        provider: Option<String>,
    },
    /// Show the projected SQLite quality report for one session
    Report {
        /// Provider ID
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// Provider session ID
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
    },
    /// Recompute stale flags for projected SQLite session snapshots
    RefreshStale,
    /// Rebuild stale projected SQLite session snapshots from provider sources
    ReprojectStale {
        /// Limit reprojection to one provider
        #[arg(short, long, value_name = "PROVIDER")]
        provider: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum BackupCommands {
    /// List registered backups
    List {
        /// Filter by mutation operation ID
        #[arg(long, value_name = "OPERATION_ID")]
        operation: Option<String>,
        /// Filter by provider ID
        #[arg(short, long, value_name = "PROVIDER")]
        provider: Option<String>,
        /// Filter by provider session ID
        #[arg(long, value_name = "SESSION_ID")]
        session: Option<String>,
        /// Filter by latest restore status: running, success, failed
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,
        /// Maximum records to return
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
        /// Print records as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show one registered backup
    Show {
        /// Registered backup ID
        #[arg(value_name = "BACKUP_ID")]
        backup_id: String,
        /// Print the record as JSON
        #[arg(long)]
        json: bool,
    },
    /// Restore one registered native session backup
    Restore {
        /// Registered backup ID
        #[arg(value_name = "BACKUP_ID")]
        backup_id: String,
    },
}

#[derive(Subcommand)]
pub enum DatabaseCommands {
    /// Create and register a consistent memorph.db backup bundle
    Backup {
        /// Directory that will contain the generated backup bundle
        #[arg(long, value_name = "DIR")]
        output_dir: Option<String>,
        /// Print the report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Verify a database backup bundle without changing local state
    Verify {
        /// Database backup bundle directory
        #[arg(value_name = "BUNDLE")]
        bundle: String,
        /// Print the report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Restore memorph.db from a verified backup bundle
    Restore {
        /// Database backup bundle directory
        #[arg(value_name = "BUNDLE")]
        bundle: String,
        /// Confirm replacement of the current memorph management database
        #[arg(long)]
        confirm: bool,
        /// Print the report as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum ArtifactCommands {
    /// Inspect registered artifacts and unregistered event payload files
    Inspect {
        /// Print the report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Plan or apply cleanup of detached and orphan event payload artifacts
    Cleanup {
        /// Minimum artifact age in hours
        #[arg(long, default_value = "168", value_name = "HOURS")]
        retention_hours: u64,
        /// Execute deletion; without this flag only a plan is returned
        #[arg(long)]
        apply: bool,
        /// Print the report as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum CompressionCommands {
    /// List compressed session archives
    List,
    /// List provider compression support profiles
    Providers,
    /// Print the archive retrieval tool specification as JSON
    ToolSpec,
    /// Print query-first retrieval instructions for an archive ref
    Instructions {
        /// Archive ref, for example memorph-archive://...
        #[arg(value_name = "ARCHIVE_REF")]
        archive_ref: String,
    },
    /// Restore a compressed archive to file(s)
    Restore {
        /// Archive ref, for example memorph-archive://...
        #[arg(value_name = "ARCHIVE_REF")]
        archive_ref: String,
        /// Output filename prefix (default: archive canonical ID)
        #[arg(short, long, value_name = "PREFIX")]
        output: Option<String>,
        /// Output format: json, md, html, morph, both (default: json)
        #[arg(short, long, value_name = "FORMAT", default_value = "json")]
        format: String,
    },
    /// Retrieve archived original events and print them as JSON
    Retrieve {
        /// Archive ref, for example memorph-archive://...
        #[arg(value_name = "ARCHIVE_REF")]
        archive_ref: String,
        /// Search query for retrieving only matching archived events
        #[arg(short, long, value_name = "QUERY")]
        query: Option<String>,
        /// Maximum matching events to return when --query is provided
        #[arg(long, value_name = "N")]
        max_results: Option<usize>,
    },
    /// Expand compressed segments in a canonical export file to file(s)
    Expand {
        /// Path to .json/.md/.html/.morph canonical export file
        #[arg(value_name = "FILE")]
        file: String,
        /// Output filename prefix (default: input filename with _expanded)
        #[arg(short, long, value_name = "PREFIX")]
        output: Option<String>,
        /// Output format: json, md, html, morph, both (default: json)
        #[arg(short, long, value_name = "FORMAT", default_value = "json")]
        format: String,
    },
    /// Dry-run active compression and show candidate ranges without applying
    Plan {
        /// Source provider ID, or source provider hint when using --file
        #[arg(value_name = "SOURCE")]
        source_provider_id: String,
        /// Target provider ID for projection-aware planning
        #[arg(value_name = "TARGET")]
        target_provider_id: String,
        /// Source session ID to load from SOURCE
        #[arg(short, long, value_name = "SESSION_ID", conflicts_with = "file")]
        session_id: Option<String>,
        /// Canonical export file to plan from instead of provider storage
        #[arg(long, value_name = "FILE", conflicts_with = "session_id")]
        file: Option<String>,
        /// Number of latest message events to protect from compression
        #[arg(long, value_name = "N")]
        protect_recent_message_events: Option<usize>,
        /// Minimum candidate size in bytes
        #[arg(long, value_name = "BYTES")]
        min_candidate_bytes: Option<usize>,
        /// Minimum estimated savings ratio percentage
        #[arg(long, value_name = "PERCENT")]
        min_savings_ratio_percent: Option<u8>,
    },
    /// Apply active compression and write a compressed canonical export
    Apply {
        /// Source provider ID, or source provider hint when using --file
        #[arg(value_name = "SOURCE")]
        source_provider_id: String,
        /// Target provider ID for projection-aware compression
        #[arg(value_name = "TARGET")]
        target_provider_id: String,
        /// Source session ID to load from SOURCE
        #[arg(short, long, value_name = "SESSION_ID", conflicts_with = "file")]
        session_id: Option<String>,
        /// Canonical export file to compress instead of provider storage
        #[arg(long, value_name = "FILE", conflicts_with = "session_id")]
        file: Option<String>,
        /// Candidate ID to compress; repeat to select multiple. Empty selects all candidates.
        #[arg(long = "candidate-id", value_name = "ID")]
        candidate_ids: Vec<String>,
        /// Output filename prefix (default: CANONICAL_ID_active_compressed)
        #[arg(short, long, value_name = "PREFIX")]
        output: Option<String>,
        /// Output format: json, md, html, morph, both (default: json)
        #[arg(short, long, value_name = "FORMAT", default_value = "json")]
        format: String,
        /// Number of latest message events to protect from compression
        #[arg(long, value_name = "N")]
        protect_recent_message_events: Option<usize>,
        /// Minimum candidate size in bytes
        #[arg(long, value_name = "BYTES")]
        min_candidate_bytes: Option<usize>,
        /// Minimum estimated savings ratio percentage
        #[arg(long, value_name = "PERCENT")]
        min_savings_ratio_percent: Option<u8>,
    },
}

#[derive(Subcommand)]
pub enum SyncCommands {
    /// Create a sync group from an existing provider session
    Create {
        /// Source provider ID
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// Source session ID
        #[arg(value_name = "SESSION_ID")]
        session_id: String,
        /// Target provider to bind; repeat for multiple targets
        #[arg(long = "to", short = 't', value_name = "PROVIDER")]
        targets: Vec<String>,
        /// Target project directory for newly created provider sessions
        #[arg(short, long, value_name = "DIR")]
        to_dir: Option<String>,
        /// Sync group title
        #[arg(long, value_name = "TITLE")]
        title: Option<String>,
    },
    /// Add a provider holding to an existing sync group
    Bind {
        /// Sync group ID
        #[arg(value_name = "GROUP_ID")]
        group_id: String,
        /// Provider ID to bind
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// Existing provider session ID to bind; omitted creates a new provider session
        #[arg(long, short = 's', value_name = "SESSION_ID")]
        session_id: Option<String>,
        /// Target project directory for newly created provider sessions
        #[arg(short, long, value_name = "DIR")]
        to_dir: Option<String>,
    },
    /// Remove a holding from a sync group
    Unbind {
        /// Sync group ID
        #[arg(value_name = "GROUP_ID")]
        group_id: String,
        /// Holding ID from `sync list` or `sync status`
        #[arg(value_name = "HOLDING_ID")]
        holding_id: String,
    },
    /// Remove a sync group record
    Remove {
        /// Sync group ID
        #[arg(value_name = "GROUP_ID")]
        group_id: String,
        /// Also delete provider sessions when the provider supports deletion
        #[arg(long)]
        delete_provider_sessions: bool,
    },
    /// Rename a sync group title
    Rename {
        /// Sync group ID
        #[arg(value_name = "GROUP_ID")]
        group_id: String,
        /// New title
        #[arg(value_name = "TITLE")]
        title: String,
    },
    /// List sync groups and their holdings
    List,
    /// Show detailed sync group status
    Status {
        /// Optional sync group ID
        #[arg(value_name = "GROUP_ID")]
        group_id: Option<String>,
    },
    /// Push sync from a specific holding to all others, or auto-sync from latest
    Sync {
        /// Sync group ID
        #[arg(value_name = "GROUP_ID")]
        group_id: String,
        /// Holding ID to push from; omitted triggers auto-sync from latest
        #[arg(long, value_name = "HOLDING_ID")]
        from_holding: Option<String>,
    },
    /// Push sync from a specific holding to all others
    Push {
        /// Sync group ID
        #[arg(value_name = "GROUP_ID")]
        group_id: String,
        /// Holding ID to push from
        #[arg(value_name = "HOLDING_ID")]
        holding_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_sync_command_accepts_new_primary_flag() {
        let cli = Cli::parse_from(["memorph", "codex", "--sync"]);
        match cli.command {
            Some(Commands::Codex {
                sync,
                workspace,
                codex_home,
                keep,
            }) => {
                assert!(sync);
                assert_eq!(workspace, None);
                assert_eq!(codex_home, None);
                assert_eq!(keep, 5);
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn codex_sync_command_keeps_workspace_and_backup_options() {
        let cli = Cli::parse_from([
            "memorph",
            "codex",
            "--sync",
            "--workspace",
            "/tmp/repo",
            "--codex-home",
            "/tmp/.codex",
            "--keep",
            "3",
        ]);
        match cli.command {
            Some(Commands::Codex {
                sync,
                workspace,
                codex_home,
                keep,
                ..
            }) => {
                assert!(sync);
                assert_eq!(workspace.as_deref(), Some("/tmp/repo"));
                assert_eq!(codex_home.as_deref(), Some("/tmp/.codex"));
                assert_eq!(keep, 3);
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn sessions_refresh_stale_command_parses() {
        let cli = Cli::parse_from(["memorph", "sessions", "refresh-stale"]);
        match cli.command {
            Some(Commands::Sessions {
                command: SessionCommands::RefreshStale,
            }) => {}
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn database_commands_parse_complete_options() {
        let cli = Cli::parse_from([
            "memorph",
            "database",
            "backup",
            "--output-dir",
            "/tmp/backups",
            "--json",
        ]);
        match cli.command {
            Some(Commands::Database {
                command: DatabaseCommands::Backup { output_dir, json },
            }) => {
                assert_eq!(output_dir.as_deref(), Some("/tmp/backups"));
                assert!(json);
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }

        let cli = Cli::parse_from([
            "memorph",
            "database",
            "verify",
            "/tmp/backup-bundle",
            "--json",
        ]);
        match cli.command {
            Some(Commands::Database {
                command: DatabaseCommands::Verify { bundle, json },
            }) => {
                assert_eq!(bundle, "/tmp/backup-bundle");
                assert!(json);
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }

        let cli = Cli::parse_from([
            "memorph",
            "database",
            "restore",
            "/tmp/backup-bundle",
            "--confirm",
            "--json",
        ]);
        match cli.command {
            Some(Commands::Database {
                command:
                    DatabaseCommands::Restore {
                        bundle,
                        confirm,
                        json,
                    },
            }) => {
                assert_eq!(bundle, "/tmp/backup-bundle");
                assert!(confirm);
                assert!(json);
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn sessions_bootstrap_command_parses_provider() {
        let cli = Cli::parse_from(["memorph", "sessions", "bootstrap", "--provider", "opencode"]);
        match cli.command {
            Some(Commands::Sessions {
                command: SessionCommands::Bootstrap { provider },
            }) => assert_eq!(provider.as_deref(), Some("opencode")),
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn providers_command_parses_provider_and_json_output() {
        let cli = Cli::parse_from(["memorph", "providers", "codex", "--json"]);
        match cli.command {
            Some(Commands::Providers { provider, json }) => {
                assert_eq!(provider.as_deref(), Some("codex"));
                assert!(json);
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn sessions_report_command_parses_provider_and_session() {
        let cli = Cli::parse_from(["memorph", "sessions", "report", "claude", "native-1"]);
        match cli.command {
            Some(Commands::Sessions {
                command:
                    SessionCommands::Report {
                        provider,
                        session_id,
                    },
            }) => {
                assert_eq!(provider, "claude");
                assert_eq!(session_id, "native-1");
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn sessions_reproject_stale_command_parses_provider() {
        let cli = Cli::parse_from([
            "memorph",
            "sessions",
            "reproject-stale",
            "--provider",
            "claude",
        ]);
        match cli.command {
            Some(Commands::Sessions {
                command: SessionCommands::ReprojectStale { provider },
            }) => assert_eq!(provider.as_deref(), Some("claude")),
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn backups_commands_parse_query_and_restore_inputs() {
        let cli = Cli::parse_from([
            "memorph",
            "backups",
            "list",
            "--operation",
            "operation-1",
            "--provider",
            "claude",
            "--session",
            "session-1",
            "--status",
            "failed",
            "--limit",
            "20",
            "--json",
        ]);
        match cli.command {
            Some(Commands::Backups {
                command:
                    BackupCommands::List {
                        operation,
                        provider,
                        session,
                        status,
                        limit,
                        json,
                    },
            }) => {
                assert_eq!(operation.as_deref(), Some("operation-1"));
                assert_eq!(provider.as_deref(), Some("claude"));
                assert_eq!(session.as_deref(), Some("session-1"));
                assert_eq!(status.as_deref(), Some("failed"));
                assert_eq!(limit, Some(20));
                assert!(json);
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }

        let cli = Cli::parse_from(["memorph", "backups", "restore", "backup-1"]);
        match cli.command {
            Some(Commands::Backups {
                command: BackupCommands::Restore { backup_id },
            }) => assert_eq!(backup_id, "backup-1"),
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn artifacts_inspect_command_parses_json_output() {
        let cli = Cli::parse_from(["memorph", "artifacts", "inspect", "--json"]);

        match cli.command {
            Some(Commands::Artifacts {
                command: ArtifactCommands::Inspect { json },
            }) => assert!(json),
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn artifacts_cleanup_command_parses_retention_and_apply_mode() {
        let cli = Cli::parse_from([
            "memorph",
            "artifacts",
            "cleanup",
            "--retention-hours",
            "24",
            "--apply",
            "--json",
        ]);

        match cli.command {
            Some(Commands::Artifacts {
                command:
                    ArtifactCommands::Cleanup {
                        retention_hours,
                        apply,
                        json,
                    },
            }) => {
                assert_eq!(retention_hours, 24);
                assert!(apply);
                assert!(json);
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn compression_plan_accepts_file_and_policy_options() {
        let cli = Cli::parse_from([
            "memorph",
            "compression",
            "plan",
            "claude",
            "codex",
            "--file",
            "session.json",
            "--protect-recent-message-events",
            "2",
            "--min-candidate-bytes",
            "128",
            "--min-savings-ratio-percent",
            "25",
        ]);

        match cli.command {
            Some(Commands::Compression {
                command:
                    CompressionCommands::Plan {
                        source_provider_id,
                        target_provider_id,
                        session_id,
                        file,
                        protect_recent_message_events,
                        min_candidate_bytes,
                        min_savings_ratio_percent,
                    },
            }) => {
                assert_eq!(source_provider_id, "claude");
                assert_eq!(target_provider_id, "codex");
                assert_eq!(session_id, None);
                assert_eq!(file.as_deref(), Some("session.json"));
                assert_eq!(protect_recent_message_events, Some(2));
                assert_eq!(min_candidate_bytes, Some(128));
                assert_eq!(min_savings_ratio_percent, Some(25));
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn compression_apply_accepts_candidates_and_output_options() {
        let cli = Cli::parse_from([
            "memorph",
            "compression",
            "apply",
            "claude",
            "codex",
            "--file",
            "session.json",
            "--candidate-id",
            "candidate-0001",
            "--candidate-id",
            "candidate-0002",
            "--output",
            "compressed/session",
            "--format",
            "both",
        ]);

        match cli.command {
            Some(Commands::Compression {
                command:
                    CompressionCommands::Apply {
                        source_provider_id,
                        target_provider_id,
                        session_id,
                        file,
                        candidate_ids,
                        output,
                        format,
                        ..
                    },
            }) => {
                assert_eq!(source_provider_id, "claude");
                assert_eq!(target_provider_id, "codex");
                assert_eq!(session_id, None);
                assert_eq!(file.as_deref(), Some("session.json"));
                assert_eq!(
                    candidate_ids,
                    vec!["candidate-0001".to_string(), "candidate-0002".to_string()]
                );
                assert_eq!(output.as_deref(), Some("compressed/session"));
                assert_eq!(format, "both");
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn hook_bridge_command_accepts_provider_event_and_blocking() {
        let cli = Cli::parse_from([
            "memorph",
            "__hook-bridge",
            "--managed-version",
            "hook-v1",
            "--provider",
            "claude",
            "--event",
            "PreToolUse",
            "--blocking",
        ]);

        match cli.command {
            Some(Commands::HookBridge {
                managed_version,
                provider,
                event,
                blocking,
            }) => {
                assert_eq!(managed_version.as_deref(), Some("hook-v1"));
                assert_eq!(provider, "claude");
                assert_eq!(event, "PreToolUse");
                assert!(blocking);
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn compression_retrieve_accepts_archive_ref() {
        let cli = Cli::parse_from([
            "memorph",
            "compression",
            "retrieve",
            "memorph-archive://group/archive.json.gz",
            "--query",
            "needle",
            "--max-results",
            "3",
        ]);

        match cli.command {
            Some(Commands::Compression {
                command:
                    CompressionCommands::Retrieve {
                        archive_ref,
                        query,
                        max_results,
                    },
            }) => {
                assert_eq!(archive_ref, "memorph-archive://group/archive.json.gz");
                assert_eq!(query.as_deref(), Some("needle"));
                assert_eq!(max_results, Some(3));
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn compression_tool_spec_command_is_available() {
        let cli = Cli::parse_from(["memorph", "compression", "tool-spec"]);

        match cli.command {
            Some(Commands::Compression {
                command: CompressionCommands::ToolSpec,
            }) => {}
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }

    #[test]
    fn compression_instructions_accepts_archive_ref() {
        let cli = Cli::parse_from([
            "memorph",
            "compression",
            "instructions",
            "memorph-archive://group/archive.json.gz",
        ]);

        match cli.command {
            Some(Commands::Compression {
                command: CompressionCommands::Instructions { archive_ref },
            }) => {
                assert_eq!(archive_ref, "memorph-archive://group/archive.json.gz");
            }
            other => panic!("unexpected command: {:?}", other.map(|_| "other")),
        }
    }
}
