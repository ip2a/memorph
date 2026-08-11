#![recursion_limit = "256"]

use anyhow::{Context as _, Result};
use clap::Parser;
use memorph::{
    config, core, i18n,
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

fn cli_language() -> config::UiLanguage {
    config::web_preferences()
        .map(|preferences| preferences.language)
        .unwrap_or_default()
}

fn print_stat(key: &'static str, value: usize) {
    println!(
        "{}",
        i18n::format(cli_language(), key, &[("count", &value.to_string())])
    );
}

fn cli_format(key: &'static str, replacements: &[(&str, &str)]) -> String {
    i18n::format(cli_language(), key, replacements)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{}: {:#}", i18n::text(cli_language(), "cliError"), e);
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
                println!(
                    "{}",
                    i18n::format(
                        cli_language(),
                        "cliExportedFile",
                        &[("file", &file.to_string())]
                    )
                );
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
                "{}",
                i18n::format(
                    cli_language(),
                    "cliImportedSession",
                    &[
                        ("provider", &result.provider_name),
                        ("session_id", &result.new_session_id)
                    ]
                )
            );
            if let Some(cmd) = result.resume_command {
                println!(
                    "{}",
                    i18n::format(cli_language(), "cliResumeWith", &[("command", &cmd)])
                );
            }
        }

        Commands::Remove {
            provider,
            session_id,
        } => {
            let provider_name = provider_name(&provider)?;
            core::session_mutation::delete_session(&provider, &session_id, ActivityActor::Cli)?;
            println!(
                "{}",
                i18n::format(
                    cli_language(),
                    "cliRemovedSession",
                    &[("provider", &provider_name), ("session_id", &session_id)]
                )
            );
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
                "{}",
                i18n::format(
                    cli_language(),
                    "cliRenamedSession",
                    &[
                        ("provider", &result.provider_name),
                        ("session_id", &result.session_id),
                        ("title", &result.display_title)
                    ]
                )
            );
            if !result.native_updated {
                println!("{}", i18n::text(cli_language(), "cliNativeTitleNotUpdated"));
            }
            if let Some(warning) = result.warning {
                println!(
                    "{}",
                    i18n::format(cli_language(), "cliWarning", &[("message", &warning)])
                );
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

            println!(
                "{}",
                i18n::format(
                    cli_language(),
                    "cliSwitchedSession",
                    &[("from", &result.from_name), ("to", &result.to_name)]
                )
            );
            println!(
                "{}",
                i18n::format(
                    cli_language(),
                    "cliSource",
                    &[("value", &result.source_session_id)]
                )
            );
            println!(
                "{}",
                i18n::format(
                    cli_language(),
                    "cliTarget",
                    &[("value", &result.target_session_id)]
                )
            );
            if let Some(cmd) = result.resume_command {
                println!(
                    "{}",
                    i18n::format(cli_language(), "cliResume", &[("command", &cmd)])
                );
            }
        }

        Commands::Find {
            dir,
            session,
            provider,
            json,
        } => {
            if dir.is_none() && session.is_none() && provider.is_empty() {
                anyhow::bail!("{}", i18n::text(cli_language(), "cliAtLeastOneFilter"));
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
                    "\n{}",
                    i18n::format(
                        cli_language(),
                        "cliMatchesHeader",
                        &[
                            ("provider", &group.provider_name),
                            ("count", &group.sessions.len().to_string())
                        ]
                    )
                );
                for s in group.sessions.iter().take(20) {
                    let id = &s.session_id;
                    let title = truncate(s.title.as_deref().unwrap_or("(untitled)"), 40);
                    let dir = truncate(s.project_dir.as_deref().unwrap_or("(no dir)"), 40);
                    println!("  {} | {} | {}", id, title, dir);
                }
                if group.sessions.len() > 20 {
                    println!(
                        "  {}",
                        i18n::format(
                            cli_language(),
                            "cliAndMore",
                            &[("count", &(group.sessions.len() - 20).to_string())]
                        )
                    );
                }
            }

            if total_found == 0 {
                println!(
                    "{}",
                    i18n::text(cli_language(), "cliNoSessionsMatchingCriteria")
                );
            } else {
                println!(
                    "\n{}",
                    i18n::format(
                        cli_language(),
                        "cliTotalSessionsFound",
                        &[("count", &total_found.to_string())]
                    )
                );
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
    cli_format(
        "cliProviderCapabilitySummary",
        &[
            ("name", display_name),
            ("id", provider_id),
            ("scan", &serialized_enum_label(capabilities.scan_strategy)),
            ("page", &serialized_enum_label(capabilities.page_strategy)),
            (
                "storage",
                &serialized_enum_label(capabilities.storage_shape),
            ),
            ("turn", &serialized_enum_label(capabilities.turn_quality)),
            (
                "resume",
                &serialized_enum_label(capabilities.resume_quality),
            ),
            (
                "risk",
                &serialized_enum_label(capabilities.write_risk.level),
            ),
            ("operations", &provider_operations(capabilities)),
        ],
    )
}

fn provider_capability_detail(
    provider_id: &str,
    display_name: &str,
    capabilities: ProviderCapabilities,
) -> String {
    let mut lines = vec![
        cli_format(
            "cliProviderDetail",
            &[("name", display_name), ("id", provider_id)],
        ),
        cli_format(
            "cliOperations",
            &[("value", &provider_operations(capabilities))],
        ),
        cli_format(
            "cliDiscovery",
            &[
                ("scan", &serialized_enum_label(capabilities.scan_strategy)),
                ("page", &serialized_enum_label(capabilities.page_strategy)),
                (
                    "storage",
                    &serialized_enum_label(capabilities.storage_shape),
                ),
            ],
        ),
        cli_format(
            "cliTurnQuality",
            &[("value", &serialized_enum_label(capabilities.turn_quality))],
        ),
        cli_format(
            "cliResumeQuality",
            &[("value", &serialized_enum_label(capabilities.resume_quality))],
        ),
        cli_format(
            "cliWriteRisk",
            &[
                (
                    "level",
                    &serialized_enum_label(capabilities.write_risk.level),
                ),
                (
                    "multiple_files",
                    &capabilities.write_risk.multiple_files.to_string(),
                ),
                ("sqlite", &capabilities.write_risk.sqlite.to_string()),
                (
                    "sidecar_files",
                    &capabilities.write_risk.sidecar_files.to_string(),
                ),
                (
                    "index_repair",
                    &capabilities.write_risk.index_repair.to_string(),
                ),
            ],
        ),
        cli_format(
            "cliBackupSupport",
            &[
                (
                    "before_write",
                    &capabilities.backup_support.before_write.to_string(),
                ),
                ("restore", &capabilities.backup_support.restore.to_string()),
                (
                    "sync_only",
                    &capabilities.backup_support.sync_only.to_string(),
                ),
            ],
        ),
        cli_format(
            "cliActivitySupport",
            &[
                (
                    "hook_events",
                    &capabilities.activity_support.hook_events.to_string(),
                ),
                (
                    "runtime_endpoint",
                    &capabilities.activity_support.runtime_endpoint.to_string(),
                ),
                (
                    "session_activity",
                    &capabilities.activity_support.session_activity.to_string(),
                ),
            ],
        ),
        i18n::text(cli_language(), "cliImportFidelity").to_string(),
    ];
    lines.extend(provider_fidelity_lines(capabilities.import_fidelity));
    lines.push(i18n::text(cli_language(), "cliExportFidelity").to_string());
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
            print_stat("cliScannedProviders", report.scanned_providers);
            print_stat("cliFailedProviders", report.failed_providers);
            print_stat("cliDiscoveredSessions", report.discovered_sessions);
            print_stat("cliProjectedSessions", report.projected_sessions);
            print_stat("cliUnchangedSessions", report.unchanged_sessions);
            print_stat("cliMissingSources", report.missing_sources);
            print_stat("cliUnsupportedProviders", report.unsupported_providers);
            print_stat("cliFailedSessions", report.failed_sessions);
            for failure in report.failures {
                let session = failure
                    .session_id
                    .as_deref()
                    .unwrap_or(i18n::text(cli_language(), "cliProviderScan"));
                let source = failure
                    .source_path
                    .as_deref()
                    .unwrap_or(i18n::text(cli_language(), "cliNoSource"));
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
            print_stat("cliCheckedSources", report.checked_sources);
            print_stat("cliFreshSnapshots", report.fresh_snapshots);
            print_stat("cliStaleSnapshots", report.stale_snapshots);
            print_stat("cliMissingSources", report.missing_sources);
            print_stat("cliUnknownSources", report.unknown_sources);
        }
        SessionCommands::ReprojectStale { provider } => {
            let report = core::projection::reproject_stale_sessions(
                provider.as_deref(),
                ActivityActor::Cli,
            )?;
            print_stat("cliCandidateSnapshots", report.candidate_snapshots);
            print_stat("cliReprojectedSnapshots", report.reprojected_snapshots);
            print_stat("cliMissingSources", report.missing_sources);
            print_stat("cliUnsupportedProviders", report.unsupported_providers);
            print_stat("cliFailedSnapshots", report.failed_snapshots);
            for failure in report.failures {
                let source = failure
                    .source_path
                    .as_deref()
                    .unwrap_or(i18n::text(cli_language(), "cliNoSource"));
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
            println!(
                "{}",
                i18n::format(cli_language(), "cliProvider", &[("value", &provider)])
            );
            println!(
                "{}",
                i18n::format(cli_language(), "cliWorkspace", &[("value", &workspace_dir)])
            );
            print_stat("cliDiscoveredSessions", report.discovered_sessions);
            print_stat("cliProjectedSessions", report.projected_sessions);
            print_stat("cliUnchangedSessions", report.unchanged_sessions);
            print_stat("cliMissingSources", report.missing_sources);
            print_stat("cliFailedSessions", report.failed_sessions);
            for failure in report.failures {
                let session = failure
                    .session_id
                    .as_deref()
                    .unwrap_or(i18n::text(cli_language(), "cliProviderScan"));
                let source = failure
                    .source_path
                    .as_deref()
                    .unwrap_or(i18n::text(cli_language(), "cliNoSource"));
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
                        .unwrap_or_else(|| i18n::text(cli_language(), "cliNever").to_string());
                    println!(
                        "{} | {} | {} | {} | {}",
                        backup.id,
                        backup
                            .provider_id
                            .as_deref()
                            .unwrap_or(i18n::text(cli_language(), "cliNoProvider")),
                        backup
                            .provider_session_id
                            .as_deref()
                            .unwrap_or(i18n::text(cli_language(), "cliNoSession")),
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
                let none = i18n::text(cli_language(), "cliNone");
                println!("{}", cli_format("cliBackup", &[("value", &backup.id)]));
                println!(
                    "{}",
                    cli_format(
                        "cliProvider",
                        &[("value", backup.provider_id.as_deref().unwrap_or(none))]
                    )
                );
                println!(
                    "{}",
                    cli_format(
                        "cliSession",
                        &[(
                            "value",
                            backup.provider_session_id.as_deref().unwrap_or(none)
                        )]
                    )
                );
                println!(
                    "{}",
                    cli_format(
                        "cliOperation",
                        &[("value", backup.operation_id.as_deref().unwrap_or(none))]
                    )
                );
                println!(
                    "{}",
                    cli_format(
                        "cliArtifact",
                        &[("value", &backup.artifact.path.display().to_string())]
                    )
                );
                println!(
                    "{}",
                    cli_format(
                        "cliIntegrity",
                        &[("value", &view.verification.status.to_string())]
                    )
                );
                let latest_restore = view
                    .entry
                    .latest_restore
                    .as_ref()
                    .map(|record| record.status.to_string())
                    .unwrap_or_else(|| i18n::text(cli_language(), "cliNever").to_string());
                println!(
                    "{}",
                    cli_format("cliLatestRestore", &[("value", &latest_restore)])
                );
                if let Some(hint) = backup.restore_hint.as_deref() {
                    println!("{}", cli_format("cliRestoreHint", &[("value", hint)]));
                }
            }
        }
        BackupCommands::Restore { backup_id } => {
            let record = core::session_management::restore_registered_backup(
                &backup_id,
                ActivityActor::Cli,
            )?;
            println!(
                "{}",
                cli_format(
                    "cliBackupRestored",
                    &[("backup_id", &record.backup_id), ("restore_id", &record.id)]
                )
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
                println!(
                    "{}",
                    cli_format(
                        "cliDatabaseBackup",
                        &[("value", &report.backup.bundle_path.display().to_string())]
                    )
                );
                println!(
                    "{}",
                    cli_format(
                        "cliBackupId",
                        &[("value", &report.backup.manifest.backup_id)]
                    )
                );
                println!(
                    "{}",
                    cli_format("cliArtifact", &[("value", &report.artifact.id)])
                );
                println!(
                    "{}",
                    cli_format(
                        "cliSchema",
                        &[("value", &report.backup.manifest.schema_version.to_string())]
                    )
                );
                println!(
                    "{}",
                    cli_format(
                        "cliBytes",
                        &[("value", &report.backup.manifest.database_bytes.to_string())]
                    )
                );
            }
        }
        DatabaseCommands::Verify { bundle, json } => {
            let report = core::database_management::verify_database_backup(Path::new(&bundle))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "{}",
                    cli_format(
                        "cliVerifiedDatabaseBackup",
                        &[("value", &report.bundle_path.display().to_string())]
                    )
                );
                println!(
                    "{}",
                    cli_format("cliBackupId", &[("value", &report.manifest.backup_id)])
                );
                println!(
                    "{}",
                    cli_format(
                        "cliSchema",
                        &[("value", &report.manifest.schema_version.to_string())]
                    )
                );
                println!(
                    "{}",
                    cli_format("cliSqliteQuickCheck", &[("value", &report.quick_check)])
                );
                println!(
                    "{}",
                    cli_format(
                        "cliForeignKeyViolations",
                        &[("value", &report.foreign_key_violations.to_string())]
                    )
                );
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
                    "{}",
                    cli_format(
                        "cliRestoredDatabaseBackup",
                        &[(
                            "value",
                            &report.restored_backup.bundle_path.display().to_string()
                        )]
                    )
                );
                println!(
                    "{}",
                    cli_format(
                        "cliBackupId",
                        &[("value", &report.restored_backup.manifest.backup_id)]
                    )
                );
                println!(
                    "{}",
                    cli_format(
                        "cliSafetyBackup",
                        &[(
                            "value",
                            &report.safety_backup.bundle_path.display().to_string()
                        )]
                    )
                );
                println!(
                    "{}",
                    cli_format(
                        "cliSchema",
                        &[("value", &report.schema_version.to_string())]
                    )
                );
                println!(
                    "{}",
                    cli_format("cliOperation", &[("value", &report.operation_id)])
                );
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
                print_stat("cliRegisteredArtifacts", report.registered.len());
                print_stat("cliOrphanFiles", report.orphan_files.len());
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
                        "{}",
                        cli_format(
                            "cliOrphanFile",
                            &[
                                ("path", &orphan.path.display().to_string()),
                                ("bytes", &orphan.byte_size.to_string()),
                                ("managed_layout", &orphan.managed_layout.to_string()),
                            ]
                        )
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
                let cleanup_key = if report.applied {
                    "cliAppliedCleanup"
                } else {
                    "cliPlannedCleanup"
                };
                print_stat(cleanup_key, report.candidate_orphan_paths.len());
                print_stat("cliDeletedFiles", report.deleted_paths.len());
                print_stat("cliRetainedSharedFiles", report.retained_shared_paths.len());
                for failure in report.failures {
                    println!(
                        "{}",
                        cli_format("cliFailure", &[("reason", &failure.reason)])
                    );
                }
            }
        }
    }
    Ok(())
}

fn session_projection_report_text(view: &core::SessionDetailView) -> String {
    let mut lines = Vec::new();
    lines.push(cli_format("cliProvider", &[("value", &view.provider_id)]));
    lines.push(cli_format("cliSession", &[("value", &view.session_id)]));
    lines.push(cli_format("cliCanonical", &[("value", &view.canonical_id)]));
    lines.push(cli_format(
        "cliTitle",
        &[(
            "value",
            view.title
                .as_deref()
                .unwrap_or(i18n::text(cli_language(), "cliUntitled")),
        )],
    ));
    if let Some(workspace_dir) = &view.workspace_dir {
        lines.push(cli_format("cliWorkspace", &[("value", workspace_dir)]));
    }
    if let Some(source_path) = &view.source_path {
        lines.push(cli_format("cliSourceValue", &[("value", source_path)]));
    }
    lines.push(cli_format(
        "cliEvents",
        &[("value", &view.event_count.to_string())],
    ));
    lines.push(cli_format(
        "cliMessages",
        &[("value", &view.message_count.to_string())],
    ));

    let Some(report) = &view.projection_report else {
        lines.push(cli_format(
            "cliProjectionReport",
            &[("value", i18n::text(cli_language(), "cliNoProjectionReport"))],
        ));
        return lines.join("\n");
    };

    lines.push(cli_format("cliProjectionReport", &[("value", &report.id)]));
    lines.push(cli_format(
        "cliIndentedOperation",
        &[("value", &serialized_enum_label(report.operation_kind))],
    ));
    lines.push(cli_format(
        "cliIndentedStatus",
        &[("value", &serialized_enum_label(report.status))],
    ));
    lines.push(cli_format(
        "cliIndentedVersion",
        &[("value", &report.projection_version.to_string())],
    ));
    lines.push(cli_format(
        "cliCreatedAt",
        &[("value", &report.created_at.to_string())],
    ));
    if let Some(count) = report.summary.canonical_event_count {
        lines.push(cli_format(
            "cliCanonicalEvents",
            &[("value", &count.to_string())],
        ));
    }
    if let Some(direction) = report.summary.mapping_direction {
        lines.push(cli_format(
            "cliMappingDirection",
            &[("value", &serialized_enum_label(direction))],
        ));
    }
    if let Some(overall) = report.summary.mapping_overall {
        lines.push(cli_format(
            "cliMappingOverall",
            &[("value", &serialized_enum_label(overall))],
        ));
    }
    lines.push(cli_format(
        "cliFidelity",
        &[
            ("preserved", &report.summary.preserved_count.to_string()),
            ("normalized", &report.summary.normalized_count.to_string()),
            ("dropped", &report.summary.dropped_count.to_string()),
        ],
    ));
    lines.push(cli_format(
        "cliIssues",
        &[("value", &report.item_count.to_string())],
    ));
    for item in &report.items {
        let field = item
            .field_path
            .as_deref()
            .unwrap_or(i18n::text(cli_language(), "cliSessionField"));
        let reason = item
            .reason
            .as_deref()
            .unwrap_or(i18n::text(cli_language(), "cliNoReason"));
        lines.push(cli_format(
            "cliProjectionIssue",
            &[
                ("order", &item.item_order.to_string()),
                ("fidelity", &serialized_enum_label(item.fidelity)),
                ("scope", &serialized_enum_label(item.scope)),
                ("field", field),
                ("reason", reason),
            ],
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
    println!(
        "{}",
        cli_format("cliWorkspace", &[("value", &report.workspace_dir)])
    );
    println!(
        "{}",
        cli_format(
            "cliCurrentProvider",
            &[("value", &report.current_model_provider)]
        )
    );
    print_stat("cliScannedRollouts", report.scanned_rollouts);
    print_stat("cliWorkspaceSessions", report.workspace_session_count);
    print_stat("cliHiddenSessions", report.hidden_session_count);
    print_stat("cliRepairedSessions", report.repaired_session_count);
    print_stat("cliReindexedSessions", report.reindexed_session_count);
    print_stat("cliRetitledSessions", report.retitled_session_count);
    print_stat("cliUpdatedSqliteRows", report.sqlite_rows_updated);
    if report.sqlite_provider_rows_updated > 0 {
        println!(
            "{}",
            cli_format(
                "cliUpdatedSqliteProviderRows",
                &[("count", &report.sqlite_provider_rows_updated.to_string())]
            )
        );
    }
    if report.sqlite_user_event_rows_updated > 0 {
        println!(
            "{}",
            cli_format(
                "cliUpdatedSqliteUserEventRows",
                &[("count", &report.sqlite_user_event_rows_updated.to_string())]
            )
        );
    }
    if report.sqlite_cwd_rows_updated > 0 {
        println!(
            "{}",
            cli_format(
                "cliUpdatedSqliteCwdRows",
                &[("count", &report.sqlite_cwd_rows_updated.to_string())]
            )
        );
    }
    if let Some(backup_dir) = &report.backup_dir {
        println!("{}", cli_format("cliBackup", &[("value", backup_dir)]));
    }
    if report.pruned_backup_count > 0 {
        print_stat("cliPrunedBackups", report.pruned_backup_count);
    }
    if !report.skipped_rollout_files.is_empty() {
        println!(
            "{}",
            cli_format(
                "cliSkippedRolloutFiles",
                &[("count", &report.skipped_rollout_files.len().to_string())]
            )
        );
    }
    if report.touched_sessions.is_empty() {
        println!("{}", i18n::text(cli_language(), "cliNoCodexSessionsSync"));
    } else {
        println!();
        for item in report.touched_sessions {
            println!(
                "{}",
                cli_format(
                    "cliCodexTouchedSession",
                    &[
                        ("session_id", &item.session_id),
                        (
                            "title",
                            &item.title.unwrap_or_else(|| i18n::text(
                                cli_language(),
                                "cliUntitled"
                            )
                            .to_string())
                        ),
                        (
                            "previous_provider",
                            &item.previous_model_provider.unwrap_or_else(|| i18n::text(
                                cli_language(),
                                "cliNone"
                            )
                            .to_string())
                        ),
                        ("provider", &item.current_model_provider),
                        ("index_added", &item.added_to_index.to_string()),
                        ("title_fixed", &item.updated_index_title.to_string()),
                    ]
                )
            );
        }
    }
    if !report.skipped_rollout_files.is_empty() {
        println!();
        for path in report.skipped_rollout_files {
            println!("{}", cli_format("cliSkippedRollout", &[("path", &path)]));
        }
    }
}

fn run_compression_command(command: CompressionCommands) -> Result<()> {
    match command {
        CompressionCommands::List => {
            let archives = core::compression_application::list_compression_archives(None)?;
            if archives.is_empty() {
                println!("{}", i18n::text(cli_language(), "cliNoCompressionArchives"));
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
                println!(
                    "{}",
                    cli_format(
                        "cliRestoredCompressionArchive",
                        &[("file", &file.to_string())]
                    )
                );
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
                "{}",
                cli_format(
                    "cliRestoredCompressedSegments",
                    &[
                        ("segments", &result.restored_segments.to_string()),
                        ("provider", &provider_id),
                        ("session_id", &session_id),
                        ("events", &result.restored_events.to_string()),
                        ("before", &result.source_bytes_before.to_string()),
                        ("after", &result.source_bytes_after.to_string()),
                    ]
                )
            );
            if !result.remaining_archive_refs.is_empty() {
                println!("{}", i18n::text(cli_language(), "cliRemainingArchives"));
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
                println!(
                    "{}",
                    cli_format(
                        "cliExpandedCompressionSession",
                        &[("file", &file.to_string())]
                    )
                );
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
                "{}",
                cli_format(
                    "cliReplacedNativeSession",
                    &[
                        ("provider", &source_provider_id),
                        (
                            "session_id",
                            session_id
                                .as_deref()
                                .unwrap_or(i18n::text(cli_language(), "cliUnknown"))
                        ),
                        ("before", &result.source_bytes_before.to_string()),
                        ("after", &result.source_bytes_after.to_string()),
                    ]
                )
            );
            for archive_ref in &result.archive_refs {
                println!("{}", cli_format("cliArchive", &[("value", archive_ref)]));
            }
            print_active_compression_report(&result.report);
        }
    }
    Ok(())
}

fn print_active_compression_report(report: &core::active_compression::ActiveCompressionReport) {
    println!(
        "{}",
        cli_format(
            "cliActiveCompressionDryRun",
            &[
                ("source", &report.source_provider_id),
                ("target", &report.target_provider_id)
            ]
        )
    );
    println!(
        "{}",
        cli_format(
            "cliCompressionCounts",
            &[
                ("events", &report.session_event_count.to_string()),
                ("messages", &report.message_event_count.to_string()),
                (
                    "already_compressed",
                    &report.already_compressed_event_count.to_string()
                ),
            ]
        )
    );
    println!(
        "{}",
        cli_format(
            "cliCompressionEstimate",
            &[
                (
                    "original_bytes",
                    &report.original_estimated_bytes.to_string()
                ),
                (
                    "original_tokens",
                    &report.original_estimated_tokens.to_string()
                ),
                (
                    "compressed_bytes",
                    &report.compressed_estimated_bytes.to_string()
                ),
                (
                    "compressed_tokens",
                    &report.compressed_estimated_tokens.to_string()
                ),
                ("saved_bytes", &report.estimated_bytes_saved.to_string()),
                ("saved_tokens", &report.estimated_tokens_saved.to_string()),
            ]
        )
    );
    println!(
        "{}",
        cli_format(
            "cliTokenEstimator",
            &[
                (
                    "strategy",
                    &format!("{:?}", report.token_estimator.strategy)
                ),
                ("provider", &report.token_estimator.effective_provider_id),
                (
                    "chars_per_token",
                    &report
                        .token_estimator
                        .effective_chars_per_token_x100
                        .to_string()
                ),
            ]
        )
    );

    if report.candidates.is_empty() {
        println!(
            "{}",
            i18n::text(cli_language(), "cliNoCompressionCandidates")
        );
    } else {
        println!("{}", i18n::text(cli_language(), "cliCandidates"));
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
        println!("{}", i18n::text(cli_language(), "cliSkipped"));
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
            println!(
                "{}",
                cli_format("cliSharedGroupCreated", &[("id", &result.id)])
            );
            println!("{}", cli_format("cliTitle", &[("value", &result.title)]));
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
                "{}",
                cli_format(
                    "cliHoldingAdded",
                    &[
                        ("id", &holding.id),
                        ("provider", &holding.provider),
                        ("session_id", &holding.session_id)
                    ]
                )
            );
        }
        SyncCommands::Unbind {
            group_id,
            holding_id,
        } => {
            session_sync::remove_holding(&group_id, &holding_id)?;
            println!(
                "{}",
                cli_format("cliHoldingRemoved", &[("id", &holding_id)])
            );
        }
        SyncCommands::Remove {
            group_id,
            delete_provider_sessions,
        } => {
            session_sync::delete_group(&group_id, delete_provider_sessions)?;
            println!(
                "{}",
                cli_format("cliSharedGroupRemoved", &[("id", &group_id)])
            );
        }
        SyncCommands::Rename { group_id, title } => {
            session_sync::rename_group(&group_id, &title)?;
            println!(
                "{}",
                cli_format(
                    "cliSharedGroupRenamed",
                    &[("id", &group_id), ("title", &title)]
                )
            );
        }
        SyncCommands::List => {
            let groups = session_sync::list_groups()?;
            if groups.is_empty() {
                println!("{}", i18n::text(cli_language(), "cliNoSharedGroups"));
            }
            for group in groups {
                println!(
                    "\n{}",
                    cli_format(
                        "cliListGroupHeader",
                        &[
                            ("id", &group.id),
                            ("title", &group.title),
                            ("count", &group.holdings.len().to_string()),
                            ("updated", &group.updated_at.to_string())
                        ]
                    )
                );
                for holding in group.holdings {
                    let dir = holding.target_dir.as_deref().unwrap_or("-");
                    let sync_from = holding.last_sync_from.as_deref().unwrap_or("-");
                    let error = holding.last_error.as_deref().unwrap_or("-");
                    println!(
                        "  {}",
                        cli_format(
                            "cliHoldingListItem",
                            &[
                                ("id", &holding.id),
                                ("provider", &holding.provider),
                                ("session_id", &holding.session_id),
                                ("dir", dir),
                                ("sync_from", sync_from),
                                ("error", error)
                            ]
                        )
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
                println!("{}", i18n::text(cli_language(), "cliNoSharedGroups"));
            }
            for mut group in groups {
                let _ = session_sync::refresh_active_times(&mut group);
                println!(
                    "\n{}",
                    cli_format(
                        "cliStatusGroupHeader",
                        &[
                            ("id", &group.id),
                            ("title", &group.title),
                            ("created", &group.created_at.to_string()),
                            ("updated", &group.updated_at.to_string())
                        ]
                    )
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
                        "  {}",
                        cli_format(
                            "cliHoldingStatusItem",
                            &[
                                ("id", &holding.id),
                                ("provider", &holding.provider),
                                ("session_id", &holding.session_id),
                                ("active", &active),
                                ("sync_from", sync_from),
                                ("sync_at", &sync_at)
                            ]
                        )
                    );
                    if let Some(error) = holding.last_error {
                        println!(
                            "    {}",
                            cli_format("cliHoldingError", &[("error", &error)])
                        );
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
                "{}",
                cli_format(
                    "cliSyncComplete",
                    &[
                        ("source", &report.source_provider),
                        ("success", &format!("{:?}", report.success)),
                        ("count", &report.errors.len().to_string())
                    ]
                )
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
                "{}",
                cli_format(
                    "cliPushSyncComplete",
                    &[
                        ("source", &report.source_provider),
                        ("success", &format!("{:?}", report.success)),
                        ("count", &report.errors.len().to_string())
                    ]
                )
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
        fields: crate::core::SessionListFields::WithStats,
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
