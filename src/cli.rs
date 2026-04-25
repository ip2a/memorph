use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "memorph")]
#[command(about = "Convert, import, and export AI coding sessions")]
#[command(version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List sessions (default: current workspace only)
    List {
        /// Show all sessions across all workspaces
        #[arg(long)]
        all: bool,
        /// Filter to Claude Code sessions
        #[arg(long)]
        claude: bool,
        /// Filter to Codex sessions
        #[arg(long)]
        codex: bool,
        /// Filter to OpenCode sessions
        #[arg(long)]
        opencode: bool,
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
        /// Output format: json, morph, both (default: both)
        #[arg(short, long, value_name = "FORMAT", default_value = "both")]
        format: String,
    },
    /// Import a session into a target tool directory
    Import {
        /// Target provider ID
        #[arg(value_name = "PROVIDER")]
        provider: String,
        /// Path to .morph/.json file, or session ID for re-export
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
        /// Migrate from Claude Code to Codex
        #[arg(long, group = "direction")]
        claude2codex: bool,
        /// Migrate from Codex to Claude Code
        #[arg(long, group = "direction")]
        codex2claude: bool,
        /// Migrate from Claude Code to OpenCode
        #[arg(long, group = "direction")]
        claude2opencode: bool,
        /// Migrate from Codex to OpenCode
        #[arg(long, group = "direction")]
        codex2opencode: bool,
        /// Migrate from OpenCode to Claude Code
        #[arg(long, group = "direction")]
        opencode2claude: bool,
        /// Migrate from OpenCode to Codex
        #[arg(long, group = "direction")]
        opencode2codex: bool,
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
}
