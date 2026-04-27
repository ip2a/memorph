mod api;
mod cli;
mod config;
mod core;
mod format;
mod model;
mod provider;
mod providers;
mod server;
mod utils;
mod web;
mod web_assets;
mod web_modals;
mod web_support;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {:#}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::List {
            all,
            claude,
            codex,
            opencode,
        } => {
            let cwd = std::env::current_dir()?;
            let cwd_str = cwd.to_string_lossy().to_string();
            let groups = core::list_sessions(&core::SessionListParams {
                all,
                providers: selected_providers(claude, codex, opencode),
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
        }

        Commands::Export {
            provider,
            session_id,
            format,
            output,
        } => {
            let prefix = output.clone().unwrap_or_else(|| session_id.clone());
            core::export_session(&core::ExportParams {
                provider,
                session_id,
                output_prefix: output,
                format: format.clone(),
            })?;

            match format.as_str() {
                "both" => println!("Exported to {}.morph and {}.json", prefix, prefix),
                "morph" => println!("Exported to {}.morph", prefix),
                "json" => println!("Exported to {}.json", prefix),
                _ => unreachable!("core validates export format"),
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
            } else {
                anyhow::bail!("Specify one direction: --claude2codex, --codex2claude, --claude2opencode, --codex2opencode, --opencode2claude, or --opencode2codex");
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

        Commands::Serve { port, no_open } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(server::run(port, no_open))?;
        }
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

fn selected_providers(claude: bool, codex: bool, opencode: bool) -> Vec<String> {
    let mut providers = Vec::new();
    if claude {
        providers.push("claude".to_string());
    }
    if codex {
        providers.push("codex".to_string());
    }
    if opencode {
        providers.push("opencode".to_string());
    }
    providers
}

fn provider_name(provider: &str) -> Result<String> {
    providers::find_provider(provider)
        .with_context(|| format!("Unknown provider: {}", provider))
        .map(|provider| provider.name().to_string())
}
