#![recursion_limit = "256"]

use anyhow::{Context as _, Result};
use clap::Parser;
use memorph::{
    config, core,
    provider::{ProviderCapabilities, ProviderContentFidelity},
    providers,
    storage::activity_store::ActivityActor,
    storage::artifact_store::{BackupQuery, BackupRestoreStatus},
    sync as session_sync,
};
use memorph_cli::{
    cli::{
        ArtifactCommands, BackupCommands, Cli, Commands, CompressionCommands, DatabaseCommands,
        SessionCommands, SyncCommands,
    },
    server, tui, web_assets,
};
use std::path::Path;
use std::process::Command;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {:#}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    if cli.version {
        println!("memorph {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

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
            provider,
            sort,
            limit,
            offset,
            json,
        } => {
            print_session_list(all, provider, sort, limit, offset, json)?;
        }

        Commands::Export {
            provider,
            session_id,
            format,
            output,
        } => {
            let result = core::transfer::export_session(
                &core::transfer::ExportParams {
                    provider,
                    session_id,
                    output_prefix: output,
                    output_dir: None,
                    format: format.clone(),
                },
                ActivityActor::Cli,
            )?;

            for file in result.files {
                println!("Exported: {}", file);
            }
        }

        Commands::Import {
            provider,
            file_or_id,
            to_dir,
        } => {
            let result = core::transfer::import_session(
                &core::transfer::ImportParams {
                    provider,
                    file_or_id,
                    to_dir,
                },
                ActivityActor::Cli,
            )?;
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
            core::session_mutation::delete_session(&provider, &session_id, ActivityActor::Cli)?;
            println!("Removed session from {}: {}", provider_name, session_id);
        }

        Commands::Rename {
            provider,
            session_id,
            new_title,
        } => {
            let result = core::session_mutation::rename_session(
                &provider,
                &session_id,
                &new_title,
                ActivityActor::Cli,
            )?;
            println!(
                "Renamed session in {}: {} -> {}",
                result.provider_name, result.session_id, result.display_title
            );
            if !result.native_updated {
                println!("Native title was not updated.");
            }
            if let Some(warning) = result.warning {
                println!("Warning: {}", warning);
            }
        }

        Commands::Switch {
            from,
            to,
            session_id,
            to_dir,
        } => {
            let result = core::transfer::switch_session(&core::transfer::SwitchParams {
                from,
                to,
                session_id,
                to_dir,
                target_title: None,
                move_original: false,
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
            json,
        } => {
            if dir.is_none() && session.is_none() && provider.is_empty() {
                anyhow::bail!("At least one filter is required: --dir, --session, or --provider");
            }

            let groups = core::query::find_sessions(&core::query::FindParams {
                dir,
                session,
                providers: provider,
            })?;
            let total_found: usize = groups.iter().map(|group| group.sessions.len()).sum();

            if json {
                println!("{}", serde_json::to_string_pretty(&groups)?);
                return Ok(());
            }

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

        Commands::Providers { provider, json } => {
            print_provider_capabilities(provider.as_deref(), json)?;
        }

        Commands::Sessions { command } => run_session_command(command)?,

        Commands::Backups { command } => run_backup_command(command)?,

        Commands::Database { command } => run_database_command(command)?,

        Commands::Artifacts { command } => run_artifact_command(command)?,

        Commands::Sync { command } => run_sync_command(command)?,

        Commands::Compression { command } => run_compression_command(command)?,

        Commands::Web { port, no_open } => {
            let port = port.unwrap_or_else(|| {
                config::server_preferences()
                    .map(|s| s.web_port)
                    .unwrap_or_else(|e| {
                        eprintln!("Warning: failed to load config: {e}");
                        config::DEFAULT_WEB_PORT
                    })
            });
            run_web_server(port, no_open)?
        }

        Commands::Api { port } => {
            let port = port.unwrap_or_else(|| {
                config::server_preferences()
                    .map(|s| s.api_port)
                    .unwrap_or_else(|e| {
                        eprintln!("Warning: failed to load config: {e}");
                        config::DEFAULT_API_PORT
                    })
            });
            run_api_server(port)?
        }

        Commands::Tui => {
            tui::run_tui()?;
        }

        Commands::Codex {
            sync,
            workspace,
            codex_home,
            keep,
        } => {
            if !sync {
                anyhow::bail!("No Codex action selected. Use --sync.");
            }
            run_codex_sync_workspace_sessions(workspace, codex_home, keep)?;
        }

        Commands::Update => {
            update_memorph()?;
        }

        Commands::HookBridge {
            managed_version: _,
            provider,
            event,
            blocking,
        } => {
            memorph::hooks::bridge::run_blocking(memorph::hooks::bridge::BridgeRunOptions {
                provider,
                event,
                blocking,
            })?;
        }
    }

    Ok(())
}

fn print_provider_capabilities(provider_id: Option<&str>, json: bool) -> Result<()> {
    let provider_ids = match provider_id {
        Some(provider_id) => vec![provider_id.to_string()],
        None => providers::all_provider_ids()
            .iter()
            .map(|provider_id| (*provider_id).to_string())
            .collect(),
    };
    let mut entries = Vec::with_capacity(provider_ids.len());

    for provider_id in provider_ids {
        let provider = providers::find_provider(&provider_id)
            .with_context(|| format!("Unknown provider: {provider_id}"))?;
        entries.push((
            provider.id().to_string(),
            providers::catalog::display_name(provider.id()),
            provider.capabilities(),
        ));
    }

    if json {
        let values = entries
            .iter()
            .map(|(provider_id, display_name, capabilities)| {
                serde_json::json!({
                    "provider_id": provider_id,
                    "display_name": display_name,
                    "capabilities": capabilities,
                })
            })
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&values)?);
        return Ok(());
    }

    if provider_id.is_some() {
        let (provider_id, display_name, capabilities) = &entries[0];
        println!(
            "{}",
            provider_capability_detail(provider_id, display_name, *capabilities)
        );
    } else {
        for (provider_id, display_name, capabilities) in entries {
            println!(
                "{}",
                provider_capability_summary(&provider_id, &display_name, capabilities)
            );
        }
    }

    Ok(())
}

fn provider_capability_summary(
    provider_id: &str,
    display_name: &str,
    capabilities: ProviderCapabilities,
) -> String {
    format!(
        "{display_name} ({provider_id}) | scan={} | page={} | storage={} | turn={} | resume={} | risk={} | ops={}",
        serialized_enum_label(capabilities.scan_strategy),
        serialized_enum_label(capabilities.page_strategy),
        serialized_enum_label(capabilities.storage_shape),
        serialized_enum_label(capabilities.turn_quality),
        serialized_enum_label(capabilities.resume_quality),
        serialized_enum_label(capabilities.write_risk.level),
        provider_operations(capabilities),
    )
}

fn provider_capability_detail(
    provider_id: &str,
    display_name: &str,
    capabilities: ProviderCapabilities,
) -> String {
    let mut lines = vec![
        format!("Provider: {display_name} ({provider_id})"),
        format!("Operations: {}", provider_operations(capabilities)),
        format!(
            "Discovery: scan={} page={} storage={}",
            serialized_enum_label(capabilities.scan_strategy),
            serialized_enum_label(capabilities.page_strategy),
            serialized_enum_label(capabilities.storage_shape),
        ),
        format!(
            "Turn quality: {}",
            serialized_enum_label(capabilities.turn_quality)
        ),
        format!(
            "Resume quality: {}",
            serialized_enum_label(capabilities.resume_quality)
        ),
        format!(
            "Write risk: level={} multiple_files={} sqlite={} sidecar_files={} index_repair={}",
            serialized_enum_label(capabilities.write_risk.level),
            capabilities.write_risk.multiple_files,
            capabilities.write_risk.sqlite,
            capabilities.write_risk.sidecar_files,
            capabilities.write_risk.index_repair,
        ),
        format!(
            "Backup: before_write={} restore={} sync_only={}",
            capabilities.backup_support.before_write,
            capabilities.backup_support.restore,
            capabilities.backup_support.sync_only,
        ),
        format!(
            "Activity: hook_events={} runtime_endpoint={} session_activity={}",
            capabilities.activity_support.hook_events,
            capabilities.activity_support.runtime_endpoint,
            capabilities.activity_support.session_activity,
        ),
        "Import fidelity:".to_string(),
    ];
    lines.extend(provider_fidelity_lines(capabilities.import_fidelity));
    lines.push("Export fidelity:".to_string());
    lines.extend(provider_fidelity_lines(capabilities.export_fidelity));
    lines.join("\n")
}

fn provider_operations(capabilities: ProviderCapabilities) -> String {
    [
        ("scan", capabilities.scan),
        ("import", capabilities.import),
        ("export", capabilities.export),
        ("delete", capabilities.delete),
        ("rename", capabilities.rename),
        ("resume", capabilities.resume),
    ]
    .into_iter()
    .filter_map(|(name, enabled)| enabled.then_some(name))
    .collect::<Vec<_>>()
    .join(",")
}

fn provider_fidelity_lines(fidelity: ProviderContentFidelity) -> Vec<String> {
    [
        ("text", fidelity.text),
        ("thinking", fidelity.thinking),
        ("tool_call", fidelity.tool_call),
        ("tool_result", fidelity.tool_result),
        ("patch", fidelity.patch),
        ("image", fidelity.image),
        ("file", fidelity.file),
        ("compressed", fidelity.compressed),
        ("provider_payload", fidelity.provider_payload),
    ]
    .into_iter()
    .map(|(content_kind, disposition)| {
        let disposition = disposition
            .map(serialized_enum_label)
            .unwrap_or_else(|| "unknown".to_string());
        format!("  {content_kind}: {disposition}")
    })
    .collect()
}

fn run_session_command(command: SessionCommands) -> Result<()> {
    match command {
        SessionCommands::Bootstrap { provider } => {
            let report = core::projection::bootstrap_session_projections(
                provider.as_deref(),
                ActivityActor::Cli,
            )?;
            println!("Scanned providers: {}", report.scanned_providers);
            println!("Failed providers: {}", report.failed_providers);
            println!("Discovered sessions: {}", report.discovered_sessions);
            println!("Projected sessions: {}", report.projected_sessions);
            println!("Unchanged sessions: {}", report.unchanged_sessions);
            println!("Missing sources: {}", report.missing_sources);
            println!("Unsupported providers: {}", report.unsupported_providers);
            println!("Failed sessions: {}", report.failed_sessions);
            for failure in report.failures {
                let session = failure.session_id.as_deref().unwrap_or("(provider scan)");
                let source = failure.source_path.as_deref().unwrap_or("(no source)");
                println!(
                    "  {}:{} | {} | {}",
                    failure.provider_id, session, source, failure.reason
                );
            }
        }
        SessionCommands::Report {
            provider,
            session_id,
        } => {
            let view =
                core::sessions::get_session_detail_view_page(&provider, &session_id, 0, Some(0))?;
            println!("{}", session_projection_report_text(&view));
        }
        SessionCommands::RefreshStale => {
            let report = core::projection::refresh_projected_session_staleness(ActivityActor::Cli)?;
            println!("Checked sources: {}", report.checked_sources);
            println!("Fresh snapshots: {}", report.fresh_snapshots);
            println!("Stale snapshots: {}", report.stale_snapshots);
            println!("Missing sources: {}", report.missing_sources);
            println!("Unknown sources: {}", report.unknown_sources);
        }
        SessionCommands::ReprojectStale { provider } => {
            let report = core::projection::reproject_stale_sessions(
                provider.as_deref(),
                ActivityActor::Cli,
            )?;
            println!("Candidate snapshots: {}", report.candidate_snapshots);
            println!("Reprojected snapshots: {}", report.reprojected_snapshots);
            println!("Missing sources: {}", report.missing_sources);
            println!("Unsupported providers: {}", report.unsupported_providers);
            println!("Failed snapshots: {}", report.failed_snapshots);
            for failure in report.failures {
                let source = failure.source_path.as_deref().unwrap_or("(no source)");
                println!(
                    "  {}:{} | {} | {}",
                    failure.provider_id, failure.session_id, source, failure.reason
                );
            }
        }
        SessionCommands::IndexWorkspace {
            provider,
            workspace_dir,
        } => {
            let report = core::projection::index_workspace_sessions(
                &provider,
                std::path::Path::new(&workspace_dir),
                ActivityActor::Cli,
            )?;
            println!("Provider: {}", provider);
            println!("Workspace: {}", workspace_dir);
            println!("Discovered sessions: {}", report.discovered_sessions);
            println!("Projected sessions: {}", report.projected_sessions);
            println!("Unchanged sessions: {}", report.unchanged_sessions);
            println!("Missing sources: {}", report.missing_sources);
            println!("Failed sessions: {}", report.failed_sessions);
            for failure in report.failures {
                let session = failure.session_id.as_deref().unwrap_or("(provider scan)");
                let source = failure.source_path.as_deref().unwrap_or("(no source)");
                println!(
                    "  {}:{} | {} | {}",
                    failure.provider_id, session, source, failure.reason
                );
            }
        }
    }
    Ok(())
}

fn run_backup_command(command: BackupCommands) -> Result<()> {
    match command {
        BackupCommands::List {
            operation,
            provider,
            session,
            status,
            limit,
            json,
        } => {
            let restore_status = status
                .as_deref()
                .map(str::parse::<BackupRestoreStatus>)
                .transpose()?;
            let views = core::session_management::list_registered_backups(BackupQuery {
                operation_id: operation,
                provider_id: provider,
                provider_session_id: session,
                restore_status,
                limit,
            })?;
            if json {
                println!("{}", serde_json::to_string_pretty(&views)?);
            } else {
                for view in views {
                    let backup = &view.entry.backup;
                    let restore_status = view
                        .entry
                        .latest_restore
                        .as_ref()
                        .map(|record| record.status.to_string())
                        .unwrap_or_else(|| "never".to_string());
                    println!(
                        "{} | {} | {} | {} | {}",
                        backup.id,
                        backup.provider_id.as_deref().unwrap_or("(no provider)"),
                        backup
                            .provider_session_id
                            .as_deref()
                            .unwrap_or("(no session)"),
                        view.verification.status,
                        restore_status
                    );
                }
            }
        }
        BackupCommands::Show { backup_id, json } => {
            let view = core::session_management::get_registered_backup(&backup_id)?
                .with_context(|| format!("Unknown backup: {backup_id}"))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&view)?);
            } else {
                let backup = &view.entry.backup;
                println!("Backup: {}", backup.id);
                println!(
                    "Provider: {}",
                    backup.provider_id.as_deref().unwrap_or("(none)")
                );
                println!(
                    "Session: {}",
                    backup.provider_session_id.as_deref().unwrap_or("(none)")
                );
                println!(
                    "Operation: {}",
                    backup.operation_id.as_deref().unwrap_or("(none)")
                );
                println!("Artifact: {}", backup.artifact.path.display());
                println!("Integrity: {}", view.verification.status);
                println!(
                    "Latest restore: {}",
                    view.entry
                        .latest_restore
                        .as_ref()
                        .map(|record| record.status.to_string())
                        .unwrap_or_else(|| "never".to_string())
                );
                if let Some(hint) = backup.restore_hint.as_deref() {
                    println!("Restore hint: {hint}");
                }
            }
        }
        BackupCommands::Restore { backup_id } => {
            let record = core::session_management::restore_registered_backup(
                &backup_id,
                ActivityActor::Cli,
            )?;
            println!(
                "Restored backup {} successfully (restore {})",
                record.backup_id, record.id
            );
        }
    }
    Ok(())
}

fn run_database_command(command: DatabaseCommands) -> Result<()> {
    match command {
        DatabaseCommands::Backup { output_dir, json } => {
            let report = core::database_management::backup_database(
                output_dir.as_deref().map(Path::new),
                ActivityActor::Cli,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Database backup: {}", report.backup.bundle_path.display());
                println!("Backup ID: {}", report.backup.manifest.backup_id);
                println!("Artifact: {}", report.artifact.id);
                println!("Schema: {}", report.backup.manifest.schema_version);
                println!("Bytes: {}", report.backup.manifest.database_bytes);
            }
        }
        DatabaseCommands::Verify { bundle, json } => {
            let report = core::database_management::verify_database_backup(Path::new(&bundle))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Verified database backup: {}", report.bundle_path.display());
                println!("Backup ID: {}", report.manifest.backup_id);
                println!("Schema: {}", report.manifest.schema_version);
                println!("SQLite quick check: {}", report.quick_check);
                println!("Foreign key violations: {}", report.foreign_key_violations);
            }
        }
        DatabaseCommands::Restore {
            bundle,
            confirm,
            json,
        } => {
            if !confirm {
                anyhow::bail!(
                    "Database restore requires --confirm because it replaces the current memorph.db"
                );
            }
            let report = core::database_management::restore_database(
                Path::new(&bundle),
                ActivityActor::Cli,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "Restored database backup: {}",
                    report.restored_backup.bundle_path.display()
                );
                println!("Backup ID: {}", report.restored_backup.manifest.backup_id);
                println!(
                    "Safety backup: {}",
                    report.safety_backup.bundle_path.display()
                );
                println!("Schema: {}", report.schema_version);
                println!("Operation: {}", report.operation_id);
            }
        }
    }
    Ok(())
}

fn run_artifact_command(command: ArtifactCommands) -> Result<()> {
    match command {
        ArtifactCommands::Inspect { json } => {
            let report = core::management::inspect_artifacts()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Registered artifacts: {}", report.registered.len());
                println!("Orphan files: {}", report.orphan_files.len());
                for entry in report.registered {
                    println!(
                        "{} | {} | {} | {} | {}",
                        entry.manifest.id,
                        entry.manifest.artifact_kind,
                        entry.retention_state,
                        entry.verification.status,
                        entry.manifest.path.display()
                    );
                }
                for orphan in report.orphan_files {
                    println!(
                        "orphan | {} | {} bytes | managed_layout={}",
                        orphan.path.display(),
                        orphan.byte_size,
                        orphan.managed_layout
                    );
                }
            }
        }
        ArtifactCommands::Cleanup {
            retention_hours,
            apply,
            json,
        } => {
            let report =
                core::management::cleanup_artifacts(retention_hours, apply, ActivityActor::Cli)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "{} cleanup: {} orphan candidates",
                    if report.applied { "Applied" } else { "Planned" },
                    report.candidate_orphan_paths.len()
                );
                println!("Deleted files: {}", report.deleted_paths.len());
                println!(
                    "Retained shared files: {}",
                    report.retained_shared_paths.len()
                );
                for failure in report.failures {
                    println!("Failure: {}", failure.reason);
                }
            }
        }
    }
    Ok(())
}

fn session_projection_report_text(view: &core::SessionDetailView) -> String {
    let mut lines = Vec::new();
    lines.push(format!("Provider: {}", view.provider_id));
    lines.push(format!("Session: {}", view.session_id));
    lines.push(format!("Canonical: {}", view.canonical_id));
    lines.push(format!(
        "Title: {}",
        view.title.as_deref().unwrap_or("(untitled)")
    ));
    if let Some(workspace_dir) = &view.workspace_dir {
        lines.push(format!("Workspace: {}", workspace_dir));
    }
    if let Some(source_path) = &view.source_path {
        lines.push(format!("Source: {}", source_path));
    }
    lines.push(format!("Events: {}", view.event_count));
    lines.push(format!("Messages: {}", view.message_count));

    let Some(report) = &view.projection_report else {
        lines.push("Projection report: none".to_string());
        return lines.join("\n");
    };

    lines.push(format!("Projection report: {}", report.id));
    lines.push(format!(
        "  Operation: {}",
        serialized_enum_label(report.operation_kind)
    ));
    lines.push(format!(
        "  Status: {}",
        serialized_enum_label(report.status)
    ));
    lines.push(format!("  Version: {}", report.projection_version));
    lines.push(format!("  Created at: {}", report.created_at));
    if let Some(count) = report.summary.canonical_event_count {
        lines.push(format!("  Canonical events: {}", count));
    }
    if let Some(direction) = report.summary.mapping_direction {
        lines.push(format!(
            "  Mapping direction: {}",
            serialized_enum_label(direction)
        ));
    }
    if let Some(overall) = report.summary.mapping_overall {
        lines.push(format!(
            "  Mapping overall: {}",
            serialized_enum_label(overall)
        ));
    }
    lines.push(format!(
        "  Fidelity: preserved={} normalized={} dropped={}",
        report.summary.preserved_count,
        report.summary.normalized_count,
        report.summary.dropped_count
    ));
    lines.push(format!("  Issues: {}", report.item_count));
    for item in &report.items {
        let field = item.field_path.as_deref().unwrap_or("(session)");
        let reason = item.reason.as_deref().unwrap_or("(no reason)");
        lines.push(format!(
            "    {}. {} {} {} - {}",
            item.item_order,
            serialized_enum_label(item.fidelity),
            serialized_enum_label(item.scope),
            field,
            reason
        ));
    }

    lines.join("\n")
}

fn serialized_enum_label<T>(value: T) -> String
where
    T: Copy + serde::Serialize + std::fmt::Debug,
{
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(label)) => label,
        _ => format!("{:?}", value),
    }
}

fn run_codex_sync_workspace_sessions(
    workspace: Option<String>,
    codex_home: Option<String>,
    keep: usize,
) -> Result<()> {
    let report = providers::codex::management::sync_workspace_sessions(
        workspace.as_deref(),
        codex_home.as_deref().map(Path::new),
        keep,
        ActivityActor::Cli,
    )?;
    print_codex_repair_report(report);
    Ok(())
}

fn print_codex_repair_report(report: providers::codex::CodexWorkspaceRepairReport) {
    println!("Workspace: {}", report.workspace_dir);
    println!("Current provider: {}", report.current_model_provider);
    println!("Scanned rollout files: {}", report.scanned_rollouts);
    println!("Workspace sessions: {}", report.workspace_session_count);
    println!("Hidden sessions: {}", report.hidden_session_count);
    println!("Repaired sessions: {}", report.repaired_session_count);
    println!("Reindexed sessions: {}", report.reindexed_session_count);
    println!("Retitled sessions: {}", report.retitled_session_count);
    println!("Updated SQLite rows: {}", report.sqlite_rows_updated);
    if report.sqlite_provider_rows_updated > 0 {
        println!(
            "Updated SQLite provider rows: {}",
            report.sqlite_provider_rows_updated
        );
    }
    if report.sqlite_user_event_rows_updated > 0 {
        println!(
            "Updated SQLite user-event rows: {}",
            report.sqlite_user_event_rows_updated
        );
    }
    if report.sqlite_cwd_rows_updated > 0 {
        println!(
            "Updated SQLite cwd rows: {}",
            report.sqlite_cwd_rows_updated
        );
    }
    if let Some(backup_dir) = &report.backup_dir {
        println!("Backup: {}", backup_dir);
    }
    if report.pruned_backup_count > 0 {
        println!("Pruned backups: {}", report.pruned_backup_count);
    }
    if !report.skipped_rollout_files.is_empty() {
        println!(
            "Skipped rollout files: {}",
            report.skipped_rollout_files.len()
        );
    }
    if report.touched_sessions.is_empty() {
        println!("No Codex sessions needed sync.");
    } else {
        println!();
        for item in report.touched_sessions {
            println!(
                "- {} | {} | provider: {} -> {} | index_added={} | title_fixed={}",
                item.session_id,
                item.title.unwrap_or_else(|| "(untitled)".to_string()),
                item.previous_model_provider
                    .unwrap_or_else(|| "(none)".to_string()),
                item.current_model_provider,
                item.added_to_index,
                item.updated_index_title
            );
        }
    }
    if !report.skipped_rollout_files.is_empty() {
        println!();
        for path in report.skipped_rollout_files {
            println!("- skipped rollout: {}", path);
        }
    }
}

fn run_compression_command(command: CompressionCommands) -> Result<()> {
    match command {
        CompressionCommands::List => {
            let archives = core::compression_application::list_compression_archives(None)?;
            if archives.is_empty() {
                println!("No compression archives.");
            } else {
                for archive in archives {
                    println!(
                        "{} | {} -> {} | events={} | stored={}B | original={}B | ratio={:.2} | canonical={} | created={}",
                        archive.archive_ref,
                        archive.source_provider_id,
                        archive.target_provider_id,
                        archive.source_event_count,
                        archive.stored_size_bytes,
                        archive.original_size_bytes,
                        archive.compression_ratio,
                        archive.canonical_id,
                        archive.created_at.to_rfc3339()
                    );
                }
            }
        }
        CompressionCommands::Providers => {
            for support in core::compression_application::list_compression_provider_support() {
                println!(
                    "{} | source={} | target={} | default={:?}",
                    support.provider_id,
                    if support.detects_native_source {
                        "native"
                    } else {
                        "portable"
                    },
                    if support.native_target_projection {
                        "native"
                    } else {
                        "portable"
                    },
                    support.default_projection
                );
            }
        }
        CompressionCommands::ToolSpec => {
            let spec = core::compression_application::compression_retrieval_tool_spec();
            println!("{}", serde_json::to_string_pretty(&spec)?);
        }
        CompressionCommands::Instructions { archive_ref } => {
            let instructions =
                core::compression_application::compression_retrieval_instructions(&archive_ref)?;
            println!("{}", serde_json::to_string_pretty(&instructions)?);
        }
        CompressionCommands::Restore {
            archive_ref,
            output,
            format,
        } => {
            let result = core::compression_application::restore_compression_archive(
                &core::compression_application::RestoreCompressionArchiveParams {
                    archive_ref,
                    output_prefix: output,
                    format,
                },
                ActivityActor::Cli,
            )?;
            for file in result.files {
                println!("Restored compression archive: {}", file);
            }
        }
        CompressionCommands::RestoreNative {
            provider_id,
            session_id,
            archive_ref,
        } => {
            let result = core::compression_application::restore_native_compression(
                &core::compression_application::RestoreNativeCompressionParams {
                    provider_id: provider_id.clone(),
                    session_id: session_id.clone(),
                    archive_ref,
                },
                ActivityActor::Cli,
            )?;
            println!(
                "Restored {} compressed segment(s) in {}/{} ({} events, {}B -> {}B)",
                result.restored_segments,
                provider_id,
                session_id,
                result.restored_events,
                result.source_bytes_before,
                result.source_bytes_after
            );
            if !result.remaining_archive_refs.is_empty() {
                println!("Remaining archives:");
                for archive_ref in result.remaining_archive_refs {
                    println!("- {archive_ref}");
                }
            }
        }
        CompressionCommands::Retrieve {
            archive_ref,
            query,
            max_results,
        } => {
            let result = core::compression_application::retrieve_compression_archive(
                &core::compression_application::RetrieveCompressionArchiveParams {
                    archive_ref,
                    query,
                    max_results,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        CompressionCommands::Expand {
            file,
            output,
            format,
        } => {
            let result = core::compression_application::expand_compression_session(
                &core::compression_application::ExpandCompressionSessionParams {
                    file,
                    output_prefix: output,
                    format,
                },
                ActivityActor::Cli,
            )?;
            for file in result.files {
                println!("Expanded compression session: {}", file);
            }
        }
        CompressionCommands::Plan {
            source_provider_id,
            target_provider_id,
            session_id,
            file,
            protect_recent_message_events,
            min_candidate_bytes,
            min_savings_ratio_percent,
        } => {
            let mut policy = core::active_compression::ActiveCompressionPolicy::default();
            policy.mode = core::active_compression::ActiveCompressionMode::PlanOnly;
            if let Some(value) = protect_recent_message_events {
                policy.protect_recent_message_events = value;
            }
            if let Some(value) = min_candidate_bytes {
                policy.min_candidate_bytes = value;
            }
            if let Some(value) = min_savings_ratio_percent {
                policy.min_savings_ratio_percent = value;
            }
            let report = core::compression_application::active_compression_dry_run(
                &core::compression_application::ActiveCompressionDryRunParams {
                    source_provider_id,
                    target_provider_id,
                    session_id,
                    file,
                    policy,
                },
            )?;
            print_active_compression_report(&report);
        }
        CompressionCommands::Apply {
            source_provider_id,
            target_provider_id,
            session_id,
            file,
            candidate_ids,
            output,
            format,
            protect_recent_message_events,
            min_candidate_bytes,
            min_savings_ratio_percent,
        } => {
            let mut policy = core::active_compression::ActiveCompressionPolicy::default();
            policy.mode = core::active_compression::ActiveCompressionMode::Auto;
            if let Some(value) = protect_recent_message_events {
                policy.protect_recent_message_events = value;
            }
            if let Some(value) = min_candidate_bytes {
                policy.min_candidate_bytes = value;
            }
            if let Some(value) = min_savings_ratio_percent {
                policy.min_savings_ratio_percent = value;
            }
            let result = core::compression_application::active_compression_apply(
                &core::compression_application::ActiveCompressionApplyCommandParams {
                    source_provider_id: source_provider_id.clone(),
                    target_provider_id,
                    session_id: session_id.clone(),
                    file,
                    policy,
                    candidate_ids,
                    output_prefix: output,
                    format,
                },
                ActivityActor::Cli,
            )?;
            println!(
                "Replaced native session {}/{} ({}B -> {}B)",
                source_provider_id,
                session_id.as_deref().unwrap_or("(unknown)"),
                result.source_bytes_before,
                result.source_bytes_after
            );
            for archive_ref in &result.archive_refs {
                println!("Archive: {}", archive_ref);
            }
            print_active_compression_report(&result.report);
        }
    }
    Ok(())
}

fn print_active_compression_report(report: &core::active_compression::ActiveCompressionReport) {
    println!(
        "Active compression dry-run: {} -> {}",
        report.source_provider_id, report.target_provider_id
    );
    println!(
        "events={} messages={} already_compressed={}",
        report.session_event_count,
        report.message_event_count,
        report.already_compressed_event_count
    );
    println!(
        "estimated: {}B/{} tokens -> {}B/{} tokens, saved {}B/{} tokens",
        report.original_estimated_bytes,
        report.original_estimated_tokens,
        report.compressed_estimated_bytes,
        report.compressed_estimated_tokens,
        report.estimated_bytes_saved,
        report.estimated_tokens_saved
    );
    println!(
        "token estimator: {:?}, effective={} chars/token x100={}",
        report.token_estimator.strategy,
        report.token_estimator.effective_provider_id,
        report.token_estimator.effective_chars_per_token_x100
    );

    if report.candidates.is_empty() {
        println!("No compression candidates.");
    } else {
        println!("Candidates:");
        for candidate in &report.candidates {
            println!(
                "- {} {:?} events={:?} reason={:?} risk={:?} saved={}B/{} tokens",
                candidate.id,
                candidate.kind,
                candidate.event_ids,
                candidate.reason,
                candidate.risk,
                candidate.estimated_bytes_saved,
                candidate.estimated_tokens_saved
            );
        }
    }

    if !report.skipped.is_empty() {
        println!("Skipped:");
        for skipped in &report.skipped {
            println!(
                "- {} reason={:?} size={}B/{} tokens",
                skipped.event_id, skipped.reason, skipped.estimated_bytes, skipped.estimated_tokens
            );
        }
    }
}

fn run_sync_command(command: SyncCommands) -> Result<()> {
    match command {
        SyncCommands::Create {
            provider,
            session_id,
            targets,
            to_dir,
            title,
        } => {
            let result = session_sync::create_group(&session_sync::SyncCreateParams {
                provider,
                session_id,
                targets,
                to_dir,
                title,
            })?;
            println!("Sync group created: {}", result.id);
            println!("Title: {}", result.title);
            for holding in result.holdings {
                println!(
                    "  {} | {} | {}",
                    holding.id, holding.provider, holding.session_id
                );
            }
        }
        SyncCommands::Bind {
            group_id,
            provider,
            session_id,
            to_dir,
        } => {
            let holding = session_sync::add_holding(&session_sync::AddHoldingParams {
                group_id: group_id.clone(),
                provider,
                session_id,
                to_dir,
            })?;
            println!(
                "Holding added: {} | {} | {}",
                holding.id, holding.provider, holding.session_id
            );
        }
        SyncCommands::Unbind {
            group_id,
            holding_id,
        } => {
            session_sync::remove_holding(&group_id, &holding_id)?;
            println!("Holding removed: {}", holding_id);
        }
        SyncCommands::Remove {
            group_id,
            delete_provider_sessions,
        } => {
            session_sync::delete_group(&group_id, delete_provider_sessions)?;
            println!("Sync group removed: {}", group_id);
        }
        SyncCommands::Rename { group_id, title } => {
            session_sync::rename_group(&group_id, &title)?;
            println!("Sync group renamed: {} -> {}", group_id, title);
        }
        SyncCommands::List => {
            let groups = session_sync::list_groups()?;
            if groups.is_empty() {
                println!("No sync groups.");
            }
            for group in groups {
                println!(
                    "\n{} | {} | holdings={} | updated={}",
                    group.id,
                    group.title,
                    group.holdings.len(),
                    group.updated_at
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
        SyncCommands::Status { group_id } => {
            let groups = if let Some(id) = group_id {
                vec![session_sync::load_group(&id)?]
            } else {
                session_sync::list_groups()?
            };
            if groups.is_empty() {
                println!("No sync groups.");
            }
            for mut group in groups {
                let _ = session_sync::refresh_active_times(&mut group);
                println!(
                    "\n{} | {} | created={} | updated={}",
                    group.id, group.title, group.created_at, group.updated_at
                );
                for holding in group.holdings {
                    let active = holding
                        .last_active_at
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let sync_at = holding
                        .last_sync_at
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let sync_from = holding.last_sync_from.as_deref().unwrap_or("-");
                    println!(
                        "  {} | {} | {} | active_at={} | sync_at={} | sync_from={}",
                        holding.id,
                        holding.provider,
                        holding.session_id,
                        active,
                        sync_at,
                        sync_from
                    );
                    if let Some(error) = holding.last_error {
                        println!("    error={}", error);
                    }
                }
            }
        }
        SyncCommands::Sync {
            group_id,
            from_holding,
        } => {
            let report = if let Some(holding_id) = from_holding {
                session_sync::push_sync(&group_id, &holding_id, ActivityActor::Cli)?
            } else {
                session_sync::sync_to_latest(&group_id, ActivityActor::Cli)?
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
        SyncCommands::Push {
            group_id,
            holding_id,
        } => {
            let report = session_sync::push_sync(&group_id, &holding_id, ActivityActor::Cli)?;
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

fn run_web_server(port: u16, no_open: bool) -> Result<()> {
    print_web_banner();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(server::run(port, no_open, true))
}

fn run_api_server(port: u16) -> Result<()> {
    println!("Starting memorph API server.");
    println!("Use `memorph web` for the Web UI.");
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(server::run_api(port, true))
}

fn update_memorph() -> Result<()> {
    let plan = current_update_plan()?;

    println!("Detected install source: {}", plan.source.label());
    println!("Running: {}", plan.display());

    let status = Command::new(&plan.program)
        .args(&plan.args)
        .status()
        .with_context(|| format!("Failed to start update command: {}", plan.program))?;

    if !status.success() {
        anyhow::bail!("Update command failed with status: {}", status);
    }

    println!("Update complete. Run `memorph --version` or `memo --version` to verify.");
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallSource {
    Npm,
    PythonPip,
    PythonPipx,
    PythonUvTool,
}

impl InstallSource {
    fn label(self) -> &'static str {
        match self {
            InstallSource::Npm => "npm",
            InstallSource::PythonPip => "PyPI/pip",
            InstallSource::PythonPipx => "PyPI/pipx",
            InstallSource::PythonUvTool => "PyPI/uv tool",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdatePlan {
    source: InstallSource,
    program: String,
    args: Vec<String>,
}

impl UpdatePlan {
    fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_word)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn current_update_plan() -> Result<UpdatePlan> {
    let exe_path = std::env::current_exe().ok();
    let source = detect_install_source(
        std::env::var("MEMORPH_INSTALL_SOURCE").ok().as_deref(),
        exe_path.as_deref(),
        std::env::var("MEMORPH_PYTHON_PREFIX").ok().as_deref(),
        std::env::var("MEMORPH_PYTHON_EXECUTABLE").ok().as_deref(),
    )
    .with_context(|| {
        "Could not detect how memorph was installed.\n\
         Try one of these commands manually:\n\
         - npm install -g memorph@latest\n\
         - python -m pip install --upgrade memorph\n\
         - pipx upgrade memorph\n\
         - uv tool upgrade memorph"
    })?;

    Ok(update_plan_for_source(
        source,
        std::env::var("MEMORPH_PYTHON_EXECUTABLE").ok(),
    ))
}

fn update_plan_for_source(source: InstallSource, python_executable: Option<String>) -> UpdatePlan {
    match source {
        InstallSource::Npm => UpdatePlan {
            source,
            program: "npm".to_string(),
            args: vec![
                "install".to_string(),
                "-g".to_string(),
                "memorph@latest".to_string(),
            ],
        },
        InstallSource::PythonPip => UpdatePlan {
            source,
            program: python_executable.unwrap_or_else(|| "python".to_string()),
            args: vec![
                "-m".to_string(),
                "pip".to_string(),
                "install".to_string(),
                "--upgrade".to_string(),
                "memorph".to_string(),
            ],
        },
        InstallSource::PythonPipx => UpdatePlan {
            source,
            program: "pipx".to_string(),
            args: vec!["upgrade".to_string(), "memorph".to_string()],
        },
        InstallSource::PythonUvTool => UpdatePlan {
            source,
            program: "uv".to_string(),
            args: vec![
                "tool".to_string(),
                "upgrade".to_string(),
                "memorph".to_string(),
            ],
        },
    }
}

fn detect_install_source(
    source_env: Option<&str>,
    exe_path: Option<&Path>,
    python_prefix: Option<&str>,
    python_executable: Option<&str>,
) -> Option<InstallSource> {
    if let Some(source) = source_env {
        match source.to_ascii_lowercase().as_str() {
            "npm" => return Some(InstallSource::Npm),
            "python" | "pypi" | "pip" => {
                if looks_like_uv_tool(python_prefix) || looks_like_uv_tool(python_executable) {
                    return Some(InstallSource::PythonUvTool);
                }
                if looks_like_pipx(python_prefix) || looks_like_pipx(python_executable) {
                    return Some(InstallSource::PythonPipx);
                }
                return Some(InstallSource::PythonPip);
            }
            "pipx" => return Some(InstallSource::PythonPipx),
            "uv" | "uv-tool" | "uv_tool" => return Some(InstallSource::PythonUvTool),
            _ => {}
        }
    }

    let path = exe_path.map(normalize_path)?;
    if path.contains("/node_modules/") && path.contains("memorph-bin") {
        return Some(InstallSource::Npm);
    }
    if path.contains("/site-packages/") && path.contains("memorph_bin") {
        return Some(InstallSource::PythonPip);
    }
    None
}

fn looks_like_uv_tool(value: Option<&str>) -> bool {
    value
        .map(|value| normalize_str_path(value).contains("/uv/tools/"))
        .unwrap_or(false)
}

fn looks_like_pipx(value: Option<&str>) -> bool {
    value
        .map(|value| normalize_str_path(value).contains("/pipx/venvs/"))
        .unwrap_or(false)
}

fn normalize_path(path: &Path) -> String {
    normalize_str_path(&path.to_string_lossy())
}

fn normalize_str_path(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn shell_word(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | ':' | '@'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn print_web_banner() {
    println!("{}", web_assets::MEMORPH_ASCII);
    println!();
    println!("Starting memorph Web UI.");
    println!("Command: memorph web");
    println!("Need API only? Use `memorph api`.");
    println!();
}

fn print_session_list(
    all: bool,
    providers: Vec<String>,
    sort: memorph_cli::cli::ListSort,
    limit: Option<usize>,
    offset: usize,
    json: bool,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let cwd_str = cwd.to_string_lossy().to_string();
    let groups = core::projection::list_sessions(&core::SessionListParams {
        all,
        providers,
        cwd: Some(cwd_str.clone()),
        include_message_counts: true,
        limit,
        offset: Some(offset),
        sort: match sort {
            memorph_cli::cli::ListSort::Recent => core::SessionListSort::Recent,
            memorph_cli::cli::ListSort::Title => core::SessionListSort::Title,
        },
    })?;
    let total_shown: usize = groups.iter().map(|group| group.sessions.len()).sum();

    if json {
        println!("{}", serde_json::to_string_pretty(&groups)?);
        return Ok(());
    }

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
            let stale = if s.stale { " | stale" } else { "" };
            println!("  {} | {} | {}{}", id, title, dir, stale);
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

fn provider_name(provider: &str) -> Result<String> {
    providers::find_provider(provider)
        .with_context(|| format!("Unknown provider: {}", provider))
        .map(|provider| provider.name().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use memorph::session::{Event, Fidelity, MappingDirection};
    use memorph::session_projection::{
        ProjectionFidelity, ProjectionItemScope, ProjectionOperationKind, ProjectionStatus,
    };
    use memorph::storage::session_state::ResolvedLocalSessionState;
    use std::path::PathBuf;

    #[test]
    fn detects_install_source_from_wrapper_env() {
        assert_eq!(
            detect_install_source(Some("npm"), None, None, None),
            Some(InstallSource::Npm)
        );
        assert_eq!(
            detect_install_source(
                Some("python"),
                None,
                Some("/Users/me/.local/share/uv/tools/memorph"),
                Some("/Users/me/.local/share/uv/tools/memorph/bin/python")
            ),
            Some(InstallSource::PythonUvTool)
        );
        assert_eq!(
            detect_install_source(
                Some("python"),
                None,
                Some("/Users/me/.local/pipx/venvs/memorph"),
                Some("/Users/me/.local/pipx/venvs/memorph/bin/python")
            ),
            Some(InstallSource::PythonPipx)
        );
        assert_eq!(detect_install_source(Some("cargo"), None, None, None), None);
    }

    #[test]
    fn detects_install_source_from_executable_path() {
        let npm_path =
            PathBuf::from("/usr/local/lib/node_modules/memorph-bin-darwin-arm64/bin/memorph");
        let pypi_path = PathBuf::from(
            "/venv/lib/python3.12/site-packages/memorph_bin_darwin_arm64/bin/memorph",
        );
        assert_eq!(
            detect_install_source(None, Some(&npm_path), None, None),
            Some(InstallSource::Npm)
        );
        assert_eq!(
            detect_install_source(None, Some(&pypi_path), None, None),
            Some(InstallSource::PythonPip)
        );
        let cargo_path = PathBuf::from("/Users/me/.cargo/bin/memo");
        assert_eq!(
            detect_install_source(None, Some(&cargo_path), None, None),
            None
        );
    }

    #[test]
    fn builds_update_commands_for_install_sources() {
        assert_eq!(
            update_plan_for_source(InstallSource::Npm, None),
            UpdatePlan {
                source: InstallSource::Npm,
                program: "npm".to_string(),
                args: vec![
                    "install".to_string(),
                    "-g".to_string(),
                    "memorph@latest".to_string()
                ],
            }
        );
        assert_eq!(
            update_plan_for_source(
                InstallSource::PythonPip,
                Some("/venv/bin/python".to_string()),
            ),
            UpdatePlan {
                source: InstallSource::PythonPip,
                program: "/venv/bin/python".to_string(),
                args: vec![
                    "-m".to_string(),
                    "pip".to_string(),
                    "install".to_string(),
                    "--upgrade".to_string(),
                    "memorph".to_string()
                ],
            }
        );
        assert_eq!(
            update_plan_for_source(InstallSource::PythonPipx, None).display(),
            "pipx upgrade memorph"
        );
    }

    #[test]
    fn session_projection_report_text_prints_quality_summary() {
        let view = core::SessionDetailView {
            provider_id: "claude".to_string(),
            provider_name: "Claude Code".to_string(),
            session_id: "native-1".to_string(),
            canonical_id: "canonical-1".to_string(),
            title: Some("Projected title".to_string()),
            native_title: Some("Native title".to_string()),
            display_title: None,
            workspace_dir: Some("/tmp/project".to_string()),
            created_at: None,
            last_active_at: None,
            source_path: Some("/tmp/session.jsonl".to_string()),
            resume_command: None,
            local_state: ResolvedLocalSessionState::default(),
            event_count: 4,
            message_count: 2,
            length_metrics: core::SessionLengthMetrics {
                provider_source_bytes_measured: 0,
                model_visible_bytes_measured: 0,
                estimated_tokens: 0,
                event_count: 0,
                message_count: 0,
                turn_count: 0,
                compressed_segment_count: 0,
                archive_count: 0,
            },
            stale: false,
            projection_report: Some(core::SessionProjectionReportView {
                id: "report-1".to_string(),
                provider_id: "claude".to_string(),
                source_id: Some("source-1".to_string()),
                operation_kind: ProjectionOperationKind::Import,
                projection_version: 1,
                status: ProjectionStatus::CompletedWithLoss,
                created_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
                created_at_ms: 0,
                summary: core::SessionProjectionReportSummaryView {
                    canonical_event_count: Some(4),
                    mapping_direction: Some(MappingDirection::Import),
                    mapping_overall: Some(Fidelity::Dropped),
                    preserved_count: 3,
                    normalized_count: 1,
                    dropped_count: 1,
                },
                item_count: 1,
                items: vec![core::SessionProjectionReportItemView {
                    item_order: 0,
                    fidelity: ProjectionFidelity::Dropped,
                    scope: ProjectionItemScope::ProviderPayload,
                    field_path: Some("events[0].payload".to_string()),
                    reason: Some("unsupported provider payload".to_string()),
                    details: None,
                }],
            }),
            turns: Vec::new(),
            events: Vec::<Event>::new(),
            compressed_archive_refs: Vec::new(),
        };

        let output = session_projection_report_text(&view);

        assert!(output.contains("Provider: claude"));
        assert!(output.contains("Session: native-1"));
        assert!(output.contains("Projection report: report-1"));
        assert!(output.contains("  Status: completed_with_loss"));
        assert!(output.contains("  Mapping overall: dropped"));
        assert!(output.contains("  Fidelity: preserved=3 normalized=1 dropped=1"));
        assert!(output.contains(
            "    0. dropped provider_payload events[0].payload - unsupported provider payload"
        ));
    }

    #[test]
    fn session_projection_report_text_handles_missing_report() {
        let view = core::SessionDetailView {
            provider_id: "claude".to_string(),
            provider_name: "Claude Code".to_string(),
            session_id: "native-1".to_string(),
            canonical_id: "canonical-1".to_string(),
            title: None,
            native_title: None,
            display_title: None,
            workspace_dir: None,
            created_at: None,
            last_active_at: None,
            source_path: None,
            resume_command: None,
            local_state: ResolvedLocalSessionState::default(),
            event_count: 0,
            message_count: 0,
            length_metrics: core::SessionLengthMetrics {
                provider_source_bytes_measured: 0,
                model_visible_bytes_measured: 0,
                estimated_tokens: 0,
                event_count: 0,
                message_count: 0,
                turn_count: 0,
                compressed_segment_count: 0,
                archive_count: 0,
            },
            stale: false,
            projection_report: None,
            turns: Vec::new(),
            events: Vec::<Event>::new(),
            compressed_archive_refs: Vec::new(),
        };

        let output = session_projection_report_text(&view);

        assert!(output.contains("Title: (untitled)"));
        assert!(output.contains("Projection report: none"));
    }

    #[test]
    fn provider_capability_detail_covers_every_registry_provider() {
        for provider_id in providers::ProviderRegistry::ids() {
            let provider = providers::find_provider(provider_id).unwrap();
            let output =
                provider_capability_detail(provider.id(), provider.name(), provider.capabilities());
            assert!(
                output.starts_with(&format!(
                    "Provider: {} ({})",
                    provider.name(),
                    provider.id()
                )),
                "missing CLI capability detail for {provider_id}"
            );
            assert!(output.contains("Operations: "));
            assert!(output.contains("Discovery: scan="));
            assert!(output.contains("Import fidelity:"));
            assert!(output.contains("Export fidelity:"));
        }
    }

    #[test]
    fn provider_capability_detail_exposes_quality_and_risk() {
        let capabilities = providers::find_provider("codex").unwrap().capabilities();

        let output = provider_capability_detail("codex", "Codex", capabilities);

        assert!(output.contains("Provider: Codex (codex)"));
        assert!(output.contains("Discovery: scan=indexed page=indexed_page storage=mixed"));
        assert!(output.contains("Turn quality: inferred"));
        assert!(output.contains("Resume quality: native"));
        assert!(output.contains("Write risk: level=high"));
        assert!(output.contains("Backup: before_write=true restore=true sync_only=false"));
        assert!(output.contains("  compressed: normalized"));
        assert!(output.contains("  provider_payload: dropped"));
    }

    #[test]
    fn deepseek_cli_capability_detail_matches_sqlite_contract() {
        let capabilities = providers::find_provider("deepseek").unwrap().capabilities();

        let output = provider_capability_detail("deepseek", "DeepSeek", capabilities);

        assert!(output.contains("Provider: DeepSeek (deepseek)"));
        assert!(output.contains("Discovery: scan=full_scan page=full_import storage=sqlite"));
        assert!(output.contains("Turn quality: inferred"));
        assert!(output.contains("Resume quality: native"));
        assert!(output.contains("Operations: scan,import,export,delete,rename,resume"));
        assert!(output.contains("Write risk: level=high"));
        assert!(output.contains("Backup: before_write=true restore=true sync_only=false"));
        assert!(output
            .contains("Activity: hook_events=false runtime_endpoint=false session_activity=false"));
        assert!(output.contains("  tool_call: downgraded"));
        assert!(output.contains("  provider_payload: dropped"));
    }

    #[test]
    fn gemini_cli_capability_detail_matches_current_jsonl_contract() {
        let capabilities = providers::find_provider("gemini").unwrap().capabilities();

        let output = provider_capability_detail("gemini", "Gemini", capabilities);

        assert!(output.contains("Provider: Gemini (gemini)"));
        assert!(output.contains("Discovery: scan=full_scan page=full_import storage=jsonl"));
        assert!(output.contains("Turn quality: inferred"));
        assert!(output.contains("Resume quality: native"));
        assert!(output.contains("Operations: scan,import,delete,resume"));
        assert!(output.contains("Write risk: level=medium multiple_files=true sqlite=false sidecar_files=true index_repair=false"));
        assert!(output.contains("Backup: before_write=true restore=true sync_only=false"));
        assert!(output
            .contains("Activity: hook_events=true runtime_endpoint=false session_activity=false"));
        assert!(output.contains("  tool_call: preserved"));
        assert!(output.contains("  provider_payload: unsupported"));
    }

    #[test]
    fn kiro_cli_capability_detail_matches_current_format_contract() {
        let capabilities = providers::find_provider("kiro").unwrap().capabilities();

        let output = provider_capability_detail("kiro", "Kiro", capabilities);

        assert!(output.contains("Provider: Kiro (kiro)"));
        assert!(output.contains("Discovery: scan=full_scan page=full_import storage=directory"));
        assert!(output.contains("Turn quality: exact"));
        assert!(output.contains("Resume quality: none"));
        assert!(output.contains("Operations: scan,import,delete,rename"));
        assert!(output.contains("Write risk: level=medium"));
        assert!(output.contains("Backup: before_write=true restore=true sync_only=false"));
        assert!(output
            .contains("Activity: hook_events=true runtime_endpoint=true session_activity=true"));
        assert!(output.contains("  tool_call: preserved"));
        assert!(output.contains("  provider_payload: preserved"));
    }
}
