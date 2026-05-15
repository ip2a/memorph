#![recursion_limit = "256"]

mod api;
mod cli;
mod config;
mod core;
mod format;
mod model;
mod provider;
mod providers;
mod server;
mod shared;
mod storage;
mod tui;
mod utils;
mod web;
mod web_assets;
mod web_modals;
mod web_support;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands, ShareCommands};

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {:#}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => run_interactive_menu()?,
        Some(command) => run_command(command)?,
    }

    Ok(())
}

fn run_command(command: Commands) -> Result<()> {
    match command {
        Commands::List {
            all,
            claude,
            codex,
            cursor,
            opencode,
            kiro,
        } => {
            print_session_list(
                all,
                selected_providers(claude, codex, cursor, opencode, kiro),
            )?;
        }

        Commands::Export {
            provider,
            session_id,
            format,
            output,
        } => {
            let result = core::export_session(&core::ExportParams {
                provider,
                session_id,
                output_prefix: output,
                format: format.clone(),
            })?;

            for file in result.files {
                println!("Exported: {}", file);
            }
        }

        Commands::Import {
            provider,
            file_or_id,
            to_dir,
        } => {
            let result = core::import_session(&core::ImportParams {
                provider,
                file_or_id,
                to_dir,
            })?;
            println!(
                "Imported session into {}: {}",
                result.provider_name, result.new_session_id
            );
            if let Some(cmd) = result.resume_command {
                println!("Resume with: {}", cmd);
            }
        }

        Commands::Remove {
            provider,
            session_id,
        } => {
            let provider_name = provider_name(&provider)?;
            core::delete_session(&provider, &session_id)?;
            println!("Removed session from {}: {}", provider_name, session_id);
        }

        Commands::Rename {
            provider,
            session_id,
            new_title,
        } => {
            let provider_name = provider_name(&provider)?;
            core::rename_session(&provider, &session_id, &new_title)?;
            println!(
                "Renamed session in {}: {} -> {}",
                provider_name, session_id, new_title
            );
        }

        Commands::Switch {
            claude2codex,
            codex2claude,
            claude2opencode,
            codex2opencode,
            opencode2claude,
            opencode2codex,
            cursor2opencode,
            opencode2cursor,
            claude2kiro,
            kiro2claude,
            codex2kiro,
            kiro2codex,
            opencode2kiro,
            kiro2opencode,
            cursor2kiro,
            kiro2cursor,
            session_id,
            to_dir,
        } => {
            let (from, to) = if claude2codex {
                ("claude", "codex")
            } else if codex2claude {
                ("codex", "claude")
            } else if claude2opencode {
                ("claude", "opencode")
            } else if codex2opencode {
                ("codex", "opencode")
            } else if opencode2claude {
                ("opencode", "claude")
            } else if opencode2codex {
                ("opencode", "codex")
            } else if cursor2opencode {
                ("cursor", "opencode")
            } else if opencode2cursor {
                ("opencode", "cursor")
            } else if claude2kiro {
                ("claude", "kiro")
            } else if kiro2claude {
                ("kiro", "claude")
            } else if codex2kiro {
                ("codex", "kiro")
            } else if kiro2codex {
                ("kiro", "codex")
            } else if opencode2kiro {
                ("opencode", "kiro")
            } else if kiro2opencode {
                ("kiro", "opencode")
            } else if cursor2kiro {
                ("cursor", "kiro")
            } else if kiro2cursor {
                ("kiro", "cursor")
            } else {
                anyhow::bail!("Specify one direction, for example --claude2codex, --opencode2cursor, --claude2kiro, or --kiro2opencode");
            };

            let result = core::switch_session(&core::SwitchParams {
                from: from.to_string(),
                to: to.to_string(),
                session_id,
                to_dir,
            })?;

            println!("Switched from {} to {}", result.from_name, result.to_name);
            println!("  Source: {}", result.source_session_id);
            println!("  Target: {}", result.target_session_id);
            if let Some(cmd) = result.resume_command {
                println!("  Resume: {}", cmd);
            }
        }

        Commands::Find {
            dir,
            session,
            provider,
        } => {
            if dir.is_none() && session.is_none() && provider.is_empty() {
                anyhow::bail!("At least one filter is required: --dir, --session, or --provider");
            }

            let groups = core::find_sessions(&core::FindParams {
                dir,
                session,
                providers: provider,
            })?;
            let total_found: usize = groups.iter().map(|group| group.sessions.len()).sum();

            for group in &groups {
                println!(
                    "\n{} ({} matches):",
                    group.provider_name,
                    group.sessions.len()
                );
                for s in group.sessions.iter().take(20) {
                    let id = &s.session_id;
                    let title = truncate(s.title.as_deref().unwrap_or("(untitled)"), 40);
                    let dir = truncate(s.project_dir.as_deref().unwrap_or("(no dir)"), 40);
                    println!("  {} | {} | {}", id, title, dir);
                }
                if group.sessions.len() > 20 {
                    println!("  ... and {} more", group.sessions.len() - 20);
                }
            }

            if total_found == 0 {
                println!("No sessions found matching the criteria.");
            } else {
                println!("\nTotal: {} sessions found", total_found);
            }
        }

        Commands::Share { command } => run_share_command(command)?,

        Commands::Web { port, no_open } => {
            run_web_server(port, no_open, WebCommandKind::Recommended)?
        }

        Commands::Serve { port, no_open } => run_web_server(port, no_open, WebCommandKind::Legacy)?,

        Commands::Api { port } => run_api_server(port)?,

        Commands::Tui => {
            tui::run_tui()?;
        }
    }

    Ok(())
}

fn run_share_command(command: ShareCommands) -> Result<()> {
    match command {
        ShareCommands::Create {
            provider,
            session_id,
            targets,
            to_dir,
            title,
        } => {
            let result = shared::create_group(&shared::ShareCreateParams {
                provider,
                session_id,
                targets,
                to_dir,
                title,
            })?;
            println!("Shared group created: {}", result.id);
            println!("Title: {}", result.title);
            for holding in result.holdings {
                println!(
                    "  {} | {} | {}",
                    holding.id, holding.provider, holding.session_id
                );
            }
        }
        ShareCommands::Bind {
            group_id,
            provider,
            session_id,
            to_dir,
        } => {
            let holding = shared::add_holding(
                &shared::AddHoldingParams {
                    group_id: group_id.clone(),
                    provider,
                    session_id,
                    to_dir,
                },
            )?;
            println!(
                "Holding added: {} | {} | {}",
                holding.id, holding.provider, holding.session_id
            );
        }
        ShareCommands::Unbind { group_id, holding_id } => {
            shared::remove_holding(&group_id, &holding_id)?;
            println!("Holding removed: {}", holding_id);
        }
        ShareCommands::Remove {
            group_id,
            delete_provider_sessions,
        } => {
            shared::delete_group(&group_id, delete_provider_sessions)?;
            println!("Shared group removed: {}", group_id);
        }
        ShareCommands::Rename { group_id, title } => {
            shared::rename_group(&group_id, &title)?;
            println!("Shared group renamed: {} -> {}", group_id, title);
        }
        ShareCommands::List => {
            let groups = shared::list_groups()?;
            if groups.is_empty() {
                println!("No shared groups.");
            }
            for group in groups {
                println!("\n{} | {} | holdings={} | updated={}",
                    group.id, group.title, group.holdings.len(), group.updated_at
                );
                for holding in group.holdings {
                    let dir = holding.target_dir.as_deref().unwrap_or("-");
                    let sync_from = holding.last_sync_from.as_deref().unwrap_or("-");
                    let error = holding.last_error.as_deref().unwrap_or("-");
                    println!(
                        "  {} | {} | {} | dir={} | sync_from={} | error={}",
                        holding.id, holding.provider, holding.session_id, dir, sync_from, error
                    );
                }
            }
        }
        ShareCommands::Status { group_id } => {
            let groups = if let Some(id) = group_id {
                vec![shared::load_group(&id)?]
            } else {
                shared::list_groups()?
            };
            if groups.is_empty() {
                println!("No shared groups.");
            }
            for mut group in groups {
                let _ = shared::refresh_active_times(&mut group);
                println!(
                    "\n{} | {} | created={} | updated={}",
                    group.id, group.title, group.created_at, group.updated_at
                );
                for holding in group.holdings {
                    let active = holding.last_active_at.map(|t| t.to_string()).unwrap_or_else(|| "-".to_string());
                    let sync_at = holding.last_sync_at.map(|t| t.to_string()).unwrap_or_else(|| "-".to_string());
                    let sync_from = holding.last_sync_from.as_deref().unwrap_or("-");
                    println!(
                        "  {} | {} | {} | active_at={} | sync_at={} | sync_from={}",
                        holding.id, holding.provider, holding.session_id, active, sync_at, sync_from
                    );
                    if let Some(error) = holding.last_error {
                        println!("    error={}", error);
                    }
                }
            }
        }
        ShareCommands::Sync { group_id, from_holding } => {
            let report = if let Some(holding_id) = from_holding {
                shared::push_sync(&group_id, &holding_id)?
            } else {
                shared::sync_to_latest(&group_id)?
            };
            println!(
                "Sync complete: source={} | success={:?} | errors={}",
                report.source_provider,
                report.success,
                report.errors.len()
            );
            for error in report.errors {
                eprintln!("  {}", error);
            }
        }
        ShareCommands::Push { group_id, holding_id } => {
            let report = shared::push_sync(&group_id, &holding_id)?;
            println!(
                "Push sync complete: source={} | success={:?} | errors={}",
                report.source_provider,
                report.success,
                report.errors.len()
            );
            for error in report.errors {
                eprintln!("  {}", error);
            }
        }
    }

    Ok(())
}

fn run_interactive_menu() -> Result<()> {
    tui::run_tui()
}

enum WebCommandKind {
    Recommended,
    Legacy,
}

fn run_web_server(port: u16, no_open: bool, kind: WebCommandKind) -> Result<()> {
    print_web_banner(kind);
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(server::run(port, no_open))
}

fn run_api_server(port: u16) -> Result<()> {
    println!("Starting memorph API server.");
    println!("Use `memorph web` for the Web UI.");
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(server::run_api(port))
}

fn print_web_banner(kind: WebCommandKind) {
    println!("{}", web_assets::MEMORPH_ASCII);
    println!();
    println!("Starting memorph Web UI.");
    match kind {
        WebCommandKind::Recommended => {
            println!("Recommended command: memorph web");
        }
        WebCommandKind::Legacy => {
            println!("`memorph serve` is still supported, but `memorph web` is recommended.");
        }
    }
    println!("Need API only? Use `memorph api`.");
    println!();
}

fn print_session_list(all: bool, providers: Vec<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let cwd_str = cwd.to_string_lossy().to_string();
    let groups = core::list_sessions(&core::SessionListParams {
        all,
        providers,
        cwd: Some(cwd_str.clone()),
    })?;
    let total_shown: usize = groups.iter().map(|group| group.sessions.len()).sum();

    for group in &groups {
        println!(
            "\n{} ({} sessions):",
            group.provider_name,
            group.sessions.len()
        );
        for s in group.sessions.iter().take(20) {
            let id = &s.session_id;
            let title = truncate(s.title.as_deref().unwrap_or("(untitled)"), 40);
            let dir = truncate(s.project_dir.as_deref().unwrap_or("(no dir)"), 40);
            println!("  {} | {} | {}", id, title, dir);
        }
        if group.sessions.len() > 20 {
            println!("  ... and {} more", group.sessions.len() - 20);
        }
    }

    if groups.is_empty() {
        if all {
            println!("No sessions found.");
        } else {
            println!(
                "No sessions found in current workspace: {}\nUse --all to show all sessions.",
                cwd_str
            );
        }
    } else {
        println!("\nTotal: {} sessions shown", total_shown);
    }

    Ok(())
}

fn truncate(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let mut result: String = s.chars().take(max_chars - 3).collect();
        result.push_str("...");
        result
    }
}

fn selected_providers(
    claude: bool,
    codex: bool,
    cursor: bool,
    opencode: bool,
    kiro: bool,
) -> Vec<String> {
    let mut providers = Vec::new();
    if claude {
        providers.push("claude".to_string());
    }
    if codex {
        providers.push("codex".to_string());
    }
    if cursor {
        providers.push("cursor".to_string());
    }
    if opencode {
        providers.push("opencode".to_string());
    }
    if kiro {
        providers.push("kiro".to_string());
    }
    providers
}

fn provider_name(provider: &str) -> Result<String> {
    providers::find_provider(provider)
        .with_context(|| format!("Unknown provider: {}", provider))
        .map(|provider| provider.name().to_string())
}
