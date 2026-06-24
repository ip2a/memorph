#![recursion_limit = "256"]

use anyhow::{Context, Result};
use clap::Parser;
use memorph::{
    cli::{
        Cli, Commands, CompressionCommands, LegacyCodexToolCommands, LegacyToolCommands,
        SyncCommands,
    },
    core, provider_features, providers, server, sync as session_sync, tui, web_assets,
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
        Commands::List { all, provider } => {
            print_session_list(all, provider)?;
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
            let result = core::rename_session(&provider, &session_id, &new_title)?;
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
            let result = core::switch_session(&core::SwitchParams {
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

        Commands::Sync { command } => run_sync_command(command)?,

        Commands::Compression { command } => run_compression_command(command)?,

        Commands::Web { port, no_open } => {
            run_web_server(port, no_open, WebCommandKind::Recommended)?
        }

        Commands::Serve { port, no_open } => run_web_server(port, no_open, WebCommandKind::Legacy)?,

        Commands::Api { port } => run_api_server(port)?,

        Commands::Tui => {
            tui::run_tui()?;
        }

        Commands::Codex {
            sync,
            repair_workspace_sessions,
            workspace,
            codex_home,
            keep,
        } => {
            if !sync && !repair_workspace_sessions {
                anyhow::bail!("No Codex action selected. Use --sync.");
            }
            run_codex_sync_workspace_sessions(workspace, codex_home, keep)?;
        }

        Commands::LegacyTool { command } => run_legacy_tool_command(command)?,

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

fn run_legacy_tool_command(command: LegacyToolCommands) -> Result<()> {
    match command {
        LegacyToolCommands::Codex { command } => match command {
            LegacyCodexToolCommands::RepairWorkspaceSessions { workspace } => {
                let output = provider_features::run_provider_feature(
                    "codex",
                    "repair_workspace_sessions",
                    provider_features::ProviderFeatureContext { workspace },
                )?;
                match output {
                    provider_features::ProviderFeatureOutput::CodexWorkspaceRepair(report) => {
                        print_codex_repair_report(report)
                    }
                    provider_features::ProviderFeatureOutput::HookOperation(report) => {
                        println!(
                            "{} {}: {:?}",
                            report.provider, report.operation, report.status.status
                        );
                        if let Some(message) = report.message {
                            println!("{}", message);
                        }
                    }
                }
            }
        },
    }

    Ok(())
}

fn run_codex_sync_workspace_sessions(
    workspace: Option<String>,
    codex_home: Option<String>,
    keep: usize,
) -> Result<()> {
    let report = providers::codex::sync_workspace_sessions(
        workspace.as_deref(),
        codex_home.as_deref().map(Path::new),
        keep,
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
            let archives = core::list_compression_archives()?;
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
            for support in core::list_compression_provider_support() {
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
            let spec = core::compression_retrieval_tool_spec();
            println!("{}", serde_json::to_string_pretty(&spec)?);
        }
        CompressionCommands::Instructions { archive_ref } => {
            let instructions = core::compression_retrieval_instructions(&archive_ref)?;
            println!("{}", serde_json::to_string_pretty(&instructions)?);
        }
        CompressionCommands::Restore {
            archive_ref,
            output,
            format,
        } => {
            let result =
                core::restore_compression_archive(&core::RestoreCompressionArchiveParams {
                    archive_ref,
                    output_prefix: output,
                    format,
                })?;
            for file in result.files {
                println!("Restored compression archive: {}", file);
            }
        }
        CompressionCommands::Retrieve {
            archive_ref,
            query,
            max_results,
        } => {
            let result =
                core::retrieve_compression_archive(&core::RetrieveCompressionArchiveParams {
                    archive_ref,
                    query,
                    max_results,
                })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        CompressionCommands::Expand {
            file,
            output,
            format,
        } => {
            let result = core::expand_compression_session(&core::ExpandCompressionSessionParams {
                file,
                output_prefix: output,
                format,
            })?;
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
            let report = core::active_compression_dry_run(&core::ActiveCompressionDryRunParams {
                source_provider_id,
                target_provider_id,
                session_id,
                file,
                policy,
            })?;
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
            let result =
                core::active_compression_apply(&core::ActiveCompressionApplyCommandParams {
                    source_provider_id,
                    target_provider_id,
                    session_id,
                    file,
                    policy,
                    candidate_ids,
                    output_prefix: output,
                    format,
                })?;
            for file in &result.files {
                println!("Wrote active compression session: {}", file);
            }
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
                session_sync::push_sync(&group_id, &holding_id)?
            } else {
                session_sync::sync_to_latest(&group_id)?
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
            let report = session_sync::push_sync(&group_id, &holding_id)?;
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
        include_message_counts: true,
        limit: None,
        offset: None,
        sort: core::SessionListSort::Recent,
        hook_filter: core::SessionHookFilter::All,
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

fn provider_name(provider: &str) -> Result<String> {
    providers::find_provider(provider)
        .with_context(|| format!("Unknown provider: {}", provider))
        .map(|provider| provider.name().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
