mod cli;
mod format;
mod model;
mod provider;
mod providers;
mod utils;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands};
use providers::find_provider;
use std::path::Path;

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

            // Determine which providers to query
            let mut provider_ids = Vec::new();
            if claude {
                provider_ids.push("claude");
            }
            if codex {
                provider_ids.push("codex");
            }
            if opencode {
                provider_ids.push("opencode");
            }
            // If no provider filter specified, query all
            if provider_ids.is_empty() {
                provider_ids = vec!["claude", "codex", "opencode"];
            }

            let mut total_shown = 0;
            let mut any_found = false;

            for pid in provider_ids {
                if let Some(prov) = find_provider(pid) {
                    let sessions = prov.scan_sessions()?;
                    // Filter by current workspace unless --all
                    let filtered: Vec<_> = if all {
                        sessions
                    } else {
                        sessions
                            .into_iter()
                            .filter(|s| {
                                s.project_dir
                                    .as_ref()
                                    .map(|d| d == &cwd_str)
                                    .unwrap_or(false)
                            })
                            .collect()
                    };

                    if !filtered.is_empty() {
                        any_found = true;
                        println!("\n{} ({} sessions):", prov.name(), filtered.len());
                        for s in filtered.iter().take(20) {
                            let id = &s.session_id;
                            let title = truncate(
                                s.title.as_deref().unwrap_or("(untitled)"),
                                40,
                            );
                            let dir = truncate(
                                s.project_dir
                                    .as_deref()
                                    .unwrap_or("(no dir)"),
                                40,
                            );
                            println!("  {} | {} | {}", id, title, dir);
                        }
                        if filtered.len() > 20 {
                            println!("  ... and {} more", filtered.len() - 20);
                        }
                        total_shown += filtered.len();
                    }
                }
            }

            if !any_found {
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
            let prov = find_provider(&provider)
                .with_context(|| format!("Unknown provider: {}", provider))?;

            // Find session source path
            let sessions = prov.scan_sessions()?;
            let session_meta = sessions
                .into_iter()
                .find(|s| s.session_id == session_id)
                .with_context(|| format!("Session not found: {}", session_id))?;

            let source_path = session_meta
                .source_path
                .as_deref()
                .context("Session has no source path")?;

            let mut session = prov.load_session(source_path)?;
            session.meta.source_session_id = session_id.clone();
            session.meta.source_provider = provider.clone();

            // Preserve title from scan metadata
            if session.session.title.is_none() {
                session.session.title = session_meta.title.clone();
            }

            let prefix = output.unwrap_or_else(|| session_id.clone());

            match format.as_str() {
                "both" => {
                    let morph_path = std::path::PathBuf::from(format!("{}.morph", prefix));
                    let json_path = std::path::PathBuf::from(format!("{}.json", prefix));
                    format::write_session(&morph_path, &session)?;
                    let json = serde_json::to_string_pretty(&session)?;
                    std::fs::write(&json_path, json)?;
                    println!("Exported to {}.morph and {}.json", prefix, prefix);
                }
                "morph" => {
                    let morph_path = std::path::PathBuf::from(format!("{}.morph", prefix));
                    format::write_session(&morph_path, &session)?;
                    println!("Exported to {}.morph", prefix);
                }
                "json" => {
                    let json_path = std::path::PathBuf::from(format!("{}.json", prefix));
                    let json = serde_json::to_string_pretty(&session)?;
                    std::fs::write(&json_path, json)?;
                    println!("Exported to {}.json", prefix);
                }
                _ => {
                    anyhow::bail!("Unsupported format: {}. Use 'json', 'morph', or 'both'", format);
                }
            }
        }

        Commands::Import {
            provider,
            file_or_id,
            to_dir,
        } => {
            let cwd = std::env::current_dir()?;
            let target_dir = if let Some(dir) = to_dir {
                let p = Path::new(&dir);
                if !p.exists() {
                    anyhow::bail!("Target directory does not exist: {}", dir);
                }
                p.canonicalize()?
            } else {
                cwd
            };

            // Load session: either from .morph file or from provider
            let session = if file_or_id.ends_with(".morph") || file_or_id.ends_with(".json") {
                let path = Path::new(&file_or_id);
                if file_or_id.ends_with(".morph") {
                    format::read_session(path)?
                } else {
                    let json = std::fs::read_to_string(path)?;
                    serde_json::from_str(&json)?
                }
            } else {
                let prov = find_provider(&provider)
                    .with_context(|| format!("Unknown provider: {}", provider))?;
                let sessions = prov.scan_sessions()?;
                let session_meta = sessions
                    .into_iter()
                    .find(|s| s.session_id == file_or_id)
                    .with_context(|| format!("Session not found: {}", file_or_id))?;
                let source_path = session_meta
                    .source_path
                    .as_deref()
                    .context("Session has no source path")?;
                prov.load_session(source_path)?
            };

            // Write to target provider
            let target_prov = providers::find_provider(&provider)
                .with_context(|| format!("Target provider not available: {}", provider))?;
            let new_id = target_prov.write_session(&session, &target_dir)?;
            println!("Imported session into {}: {}", target_prov.name(), new_id);
            if let Some(cmd) = providers::resume_command(&provider, &new_id) {
                println!("Resume with: {}", cmd);
            }
        }

        Commands::Remove {
            provider,
            session_id,
        } => {
            let prov = find_provider(&provider)
                .with_context(|| format!("Unknown provider: {}", provider))?;
            prov.delete_session(&session_id)?;
            println!("Removed session from {}: {}", prov.name(), session_id);
        }

        Commands::Rename {
            provider,
            session_id,
            new_title,
        } => {
            let prov = find_provider(&provider)
                .with_context(|| format!("Unknown provider: {}", provider))?;
            prov.rename_session(&session_id, &new_title)?;
            println!(
                "Renamed session in {}: {} -> {}",
                prov.name(),
                session_id,
                new_title
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

            let cwd = std::env::current_dir()?;
            let target_dir = if let Some(dir) = to_dir {
                let p = Path::new(&dir);
                if !p.exists() {
                    anyhow::bail!("Target directory does not exist: {}", dir);
                }
                p.canonicalize()?
            } else {
                cwd.clone()
            };

            // Resolve source session
            let source_prov = find_provider(from)
                .with_context(|| format!("Unknown source provider: {}", from))?;
            let sessions = source_prov.scan_sessions()?;
            let cwd_str = cwd.to_string_lossy().to_string();

            let session_meta = if let Some(id) = session_id {
                sessions
                    .into_iter()
                    .find(|s| s.session_id == id)
                    .with_context(|| format!("Session not found: {}", id))?
            } else {
                // Find the most recent session in current workspace
                let mut candidates: Vec<_> = sessions
                    .into_iter()
                    .filter(|s| {
                        s.project_dir
                            .as_ref()
                            .map(|d| d == &cwd_str)
                            .unwrap_or(false)
                    })
                    .collect();
                candidates.sort_by_key(|s| std::cmp::Reverse(s.last_active_at));
                candidates
                    .into_iter()
                    .next()
                    .with_context(|| {
                        format!(
                            "No {} session found in current workspace: {}\nUse --session-id to specify one, or run from the project directory.",
                            source_prov.name(),
                            cwd_str
                        )
                    })?
            };

            let source_path = session_meta
                .source_path
                .as_deref()
                .context("Session has no source path")?;

            let mut session = source_prov.load_session(source_path)?;
            session.meta.source_session_id = session_meta.session_id.clone();
            session.meta.source_provider = from.to_string();
            // Preserve title from scan metadata
            if session.session.title.is_none() {
                session.session.title = session_meta.title.clone();
            }

            // Write to target provider
            let target_prov = find_provider(to)
                .with_context(|| format!("Unknown target provider: {}", to))?;
            let new_id = target_prov.write_session(&session, &target_dir)?;

            println!("Switched from {} to {}", source_prov.name(), target_prov.name());
            println!("  Source: {}", session_meta.session_id);
            println!("  Target: {}", new_id);
            if let Some(cmd) = providers::resume_command(to, &new_id) {
                println!("  Resume: {}", cmd);
            }
        }

        Commands::Find {
            dir,
            session,
            provider,
        } => {
            // At least one filter is required
            if dir.is_none() && session.is_none() && provider.is_empty() {
                anyhow::bail!("At least one filter is required: --dir, --session, or --provider");
            }

            let provider_ids = if provider.is_empty() {
                vec!["claude", "codex", "opencode"]
            } else {
                provider.iter().map(|s| s.as_str()).collect()
            };

            let mut total_found = 0;

            for pid in provider_ids {
                if let Some(prov) = find_provider(pid) {
                    let sessions = prov.scan_sessions()?;
                    let filtered: Vec<_> = sessions
                        .into_iter()
                        .filter(|s| {
                            // Directory filter (fuzzy match)
                            let dir_match = dir.as_ref().map_or(true, |d| {
                                s.project_dir
                                    .as_ref()
                                    .map(|pd| pd.contains(d))
                                    .unwrap_or(false)
                            });
                            // Session filter (fuzzy match on ID or title)
                            let session_match = session.as_ref().map_or(true, |pat| {
                                s.session_id.contains(pat)
                                    || s.title.as_ref().map(|t| t.contains(pat)).unwrap_or(false)
                            });
                            dir_match && session_match
                        })
                        .collect();

                    if !filtered.is_empty() {
                        println!("\n{} ({} matches):", prov.name(), filtered.len());
                        for s in filtered.iter().take(20) {
                            let id = &s.session_id;
                            let title = truncate(
                                s.title.as_deref().unwrap_or("(untitled)"),
                                40,
                            );
                            let dir = truncate(
                                s.project_dir
                                    .as_deref()
                                    .unwrap_or("(no dir)"),
                                40,
                            );
                            println!("  {} | {} | {}", id, title, dir);
                        }
                        if filtered.len() > 20 {
                            println!("  ... and {} more", filtered.len() - 20);
                        }
                        total_found += filtered.len();
                    }
                }
            }

            if total_found == 0 {
                println!("No sessions found matching the criteria.");
            } else {
                println!("\nTotal: {} sessions found", total_found);
            }
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
