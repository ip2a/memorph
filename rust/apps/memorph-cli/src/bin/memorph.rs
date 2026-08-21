use anyhow::{Context as _, Result};
use chrono::{DateTime, Duration as ChronoDuration, Local, NaiveDate, TimeZone, Utc};
use clap::Parser;
use memorph::{
    config, core, i18n,
    provider::{ProviderCapabilities, ProviderContentFidelity},
    providers,
    storage::{activity_store::ActivityActor, local_store},
};
use memorph_cli::{
    cli::{Cli, Commands},
    server, tui, web_assets,
};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

fn cli_language() -> config::UiLanguage {
    config::web_preferences()
        .map(|preferences| preferences.language)
        .unwrap_or_default()
}

fn cli_format(key: &'static str, replacements: &[(&str, &str)]) -> String {
    i18n::format(cli_language(), key, replacements)
}

fn main() {
    let json_mode = std::env::args_os().skip(1).any(|arg| arg == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = error.exit_code();
            if json_mode {
                println!(
                    "{}",
                    serde_json::json!({"ok": false, "error": error.to_string()})
                );
            } else {
                let _ = error.print();
            }
            std::process::exit(exit_code);
        }
    };

    if let Err(error) = run(cli) {
        if json_mode {
            println!(
                "{}",
                serde_json::json!({"ok": false, "error": format!("{error:#}")})
            );
        } else {
            eprintln!("{}: {:#}", i18n::text(cli_language(), "cliError"), error);
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    if cli.version {
        println!("memorph {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    match cli.command {
        None => run_interactive_menu()?,
        Some(command) => run_command(command, cli.json)?,
    }
    Ok(())
}

fn run_command(command: Commands, json_mode: bool) -> Result<()> {
    match command {
        Commands::List {
            all,
            provider,
            sort,
            limit,
            offset,
            dir,
            session,
            title,
            text,
            since,
            before,
            min_bytes,
            max_bytes,
            providers: show_providers,
        } => {
            if show_providers {
                print_provider_capabilities(&provider, json_mode)?;
            } else {
                print_session_list(
                    all,
                    provider,
                    sort,
                    limit,
                    offset,
                    core::SessionListFilter {
                        dir,
                        session,
                        title,
                        text,
                        since_ms: since
                            .as_deref()
                            .map(parse_time_arg)
                            .transpose()
                            .context("Invalid --since value")?,
                        before_ms: before
                            .as_deref()
                            .map(parse_time_arg)
                            .transpose()
                            .context("Invalid --before value")?,
                        min_bytes: min_bytes
                            .as_deref()
                            .map(parse_size_arg)
                            .transpose()
                            .context("Invalid --min-bytes value")?,
                        max_bytes: max_bytes
                            .as_deref()
                            .map(parse_size_arg)
                            .transpose()
                            .context("Invalid --max-bytes value")?,
                    },
                    json_mode,
                )?;
            }
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
                    format,
                },
                ActivityActor::Cli,
            )?;
            if json_mode {
                println!("{}", serde_json::json!({"ok": true, "result": result}));
            } else {
                for file in result.files {
                    println!(
                        "{}",
                        i18n::format(cli_language(), "cliExportedFile", &[("file", &file)])
                    );
                }
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
            if json_mode {
                println!("{}", serde_json::json!({"ok": true, "result": result}));
            } else {
                println!(
                    "{}",
                    i18n::format(
                        cli_language(),
                        "cliImportedSession",
                        &[
                            ("provider", &result.provider_name),
                            ("session_id", &result.new_session_id),
                        ],
                    )
                );
                if let Some(command) = result.resume_command {
                    println!(
                        "{}",
                        i18n::format(cli_language(), "cliResumeWith", &[("command", &command)])
                    );
                }
            }
        }
        Commands::Remove {
            provider,
            session_id,
        } => {
            let provider_name = provider_name(&provider)?;
            core::session_mutation::delete_session(&provider, &session_id, ActivityActor::Cli)?;
            if json_mode {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "result": {"provider": provider_name, "session_id": session_id}
                    })
                );
            } else {
                println!(
                    "{}",
                    i18n::format(
                        cli_language(),
                        "cliRemovedSession",
                        &[("provider", &provider_name), ("session_id", &session_id)],
                    )
                );
            }
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
            if json_mode {
                println!("{}", serde_json::json!({"ok": true, "result": result}));
            } else {
                println!(
                    "{}",
                    i18n::format(
                        cli_language(),
                        "cliRenamedSession",
                        &[
                            ("provider", &result.provider_name),
                            ("session_id", &result.session_id),
                            ("title", &result.display_title),
                        ],
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
            if json_mode {
                println!("{}", serde_json::json!({"ok": true, "result": result}));
            } else {
                println!(
                    "{}",
                    i18n::format(
                        cli_language(),
                        "cliSwitchedSession",
                        &[("from", &result.from_name), ("to", &result.to_name)],
                    )
                );
                println!(
                    "{}",
                    i18n::format(
                        cli_language(),
                        "cliSource",
                        &[("value", &result.source_session_id)],
                    )
                );
                println!(
                    "{}",
                    i18n::format(
                        cli_language(),
                        "cliTarget",
                        &[("value", &result.target_session_id)],
                    )
                );
                if let Some(command) = result.resume_command {
                    println!(
                        "{}",
                        i18n::format(cli_language(), "cliResume", &[("command", &command)])
                    );
                }
            }
        }
        Commands::Web { port, no_open } => {
            let port = port.unwrap_or_else(|| {
                config::server_preferences()
                    .map(|preferences| preferences.web_port)
                    .unwrap_or(config::DEFAULT_WEB_PORT)
            });
            run_web_server(port, no_open)?;
        }
        Commands::Api { port } => {
            let port = port.unwrap_or_else(|| {
                config::server_preferences()
                    .map(|preferences| preferences.api_port)
                    .unwrap_or(config::DEFAULT_API_PORT)
            });
            run_api_server(port)?;
        }
        Commands::Tui => tui::run_tui()?,
        Commands::Doctor => run_doctor(json_mode)?,
        Commands::Update => {
            if let Some(result) = update_memorph(json_mode)? {
                println!("{}", serde_json::json!({"ok": true, "result": result}));
            }
        }
        Commands::HookBridge {
            managed_version: _,
            provider,
            event,
            blocking,
        } => memorph::hooks::bridge::run_blocking(memorph::hooks::bridge::BridgeRunOptions {
            provider,
            event,
            blocking,
        })?,
    }
    Ok(())
}

fn print_provider_capabilities(provider_ids: &[String], json: bool) -> Result<()> {
    let provider_ids = if provider_ids.is_empty() {
        providers::all_provider_ids()
            .iter()
            .map(|provider_id| (*provider_id).to_string())
            .collect()
    } else {
        provider_ids.to_vec()
    };
    let single_provider = provider_ids.len() == 1;
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

    if single_provider {
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

fn serialized_enum_label<T>(value: T) -> String
where
    T: Copy + serde::Serialize + std::fmt::Debug,
{
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(label)) => label,
        _ => format!("{:?}", value),
    }
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

fn update_memorph(json: bool) -> Result<Option<serde_json::Value>> {
    let plan = current_update_plan()?;

    if json {
        let output = Command::new(&plan.program)
            .args(&plan.args)
            .output()
            .with_context(|| format!("Failed to start update command: {}", plan.program))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "Update command failed with status: {}{}",
                output.status,
                if stderr.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {}", stderr.trim())
                }
            );
        }
        return Ok(Some(serde_json::json!({
            "source": plan.source.label(),
            "command": plan.display(),
            "stdout": String::from_utf8_lossy(&output.stdout).trim(),
            "stderr": String::from_utf8_lossy(&output.stderr).trim(),
        })));
    }

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
    Ok(None)
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

fn parse_time_arg(input: &str) -> Result<i64> {
    let input = input.trim();
    if input.is_empty() {
        anyhow::bail!("empty time value");
    }

    let relative = [('m', 60_u64), ('h', 60 * 60), ('d', 24 * 60 * 60)]
        .into_iter()
        .find_map(|(suffix, multiplier)| {
            input
                .strip_suffix(suffix)
                .map(|number| (number, multiplier))
        });
    if let Some((number, multiplier)) = relative.filter(|(number, _)| !number.is_empty()) {
        let seconds = number
            .parse::<u64>()
            .with_context(|| format!("invalid relative time: {input}"))?
            .checked_mul(multiplier)
            .and_then(|value| i64::try_from(value).ok())
            .context("relative time overflow")?;
        return Utc::now()
            .checked_sub_signed(ChronoDuration::seconds(seconds))
            .map(|value| value.timestamp_millis())
            .context("relative time out of range");
    }

    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return Local
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .map(|value| value.timestamp_millis())
            .context("date is outside local time range");
    }

    DateTime::parse_from_rfc3339(input)
        .map(|value| value.timestamp_millis())
        .with_context(|| format!("invalid time: {input}"))
}

fn parse_size_arg(input: &str) -> Result<u64> {
    let input = input.trim();
    if input.is_empty() {
        anyhow::bail!("empty size value");
    }
    let (number, multiplier) = match input.as_bytes().last().copied() {
        Some(b'K' | b'k') => (&input[..input.len() - 1], 1024_u64),
        Some(b'M' | b'm') => (&input[..input.len() - 1], 1024_u64.pow(2)),
        Some(b'G' | b'g') => (&input[..input.len() - 1], 1024_u64.pow(3)),
        _ => (input, 1),
    };
    let value: u64 = number
        .parse()
        .with_context(|| format!("invalid size: {input}"))?;
    value.checked_mul(multiplier).context("size overflow")
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    version: String,
    install_source: Option<String>,
    database: DoctorDatabase,
    providers: Vec<DoctorProvider>,
    orphan_artifact_manifests: usize,
}

#[derive(Debug, Serialize)]
struct DoctorDatabase {
    path: String,
    size_bytes: u64,
    sessions: usize,
    snapshots: usize,
    stale_snapshots: usize,
}

#[derive(Debug, Serialize)]
struct DoctorProvider {
    id: String,
    display_name: String,
    on_disk_sessions: usize,
    error: Option<String>,
}

fn run_doctor(json: bool) -> Result<()> {
    let path = local_store::database_path()?;
    let conn = local_store::open_database()?;
    let count = |sql: &str| -> Result<usize> {
        conn.query_row(sql, [], |row| row.get::<_, i64>(0))
            .map(|value| value.max(0) as usize)
            .with_context(|| format!("Failed to query doctor count: {sql}"))
    };
    let metadata = std::fs::metadata(&path).ok();
    let database = DoctorDatabase {
        path: path.display().to_string(),
        size_bytes: metadata.map(|value| value.len()).unwrap_or(0),
        sessions: count("SELECT COUNT(*) FROM sessions")?,
        snapshots: count("SELECT COUNT(*) FROM session_snapshots")?,
        stale_snapshots: count("SELECT COUNT(*) FROM session_snapshots WHERE stale = 1")?,
    };
    let orphan_artifact_manifests = count(
        "SELECT COUNT(*)
         FROM artifact_manifests am
         WHERE am.session_id IS NOT NULL
           AND NOT EXISTS (
             SELECT 1 FROM sessions s
             WHERE s.id = am.session_id AND s.deleted_at_ms IS NULL
           )",
    )?;
    drop(conn);

    let install_source = detect_install_source(
        std::env::var("MEMORPH_INSTALL_SOURCE").ok().as_deref(),
        std::env::current_exe().ok().as_deref(),
        std::env::var("MEMORPH_PYTHON_PREFIX").ok().as_deref(),
        std::env::var("MEMORPH_PYTHON_EXECUTABLE").ok().as_deref(),
    )
    .map(|source| source.label().to_string());
    let providers = providers::all_provider_ids()
        .iter()
        .map(|id| {
            let display_name = providers::catalog::display_name(id);
            match providers::find_provider(id) {
                Some(provider) => match provider.scan_sessions_lightweight() {
                    Ok(sessions) => DoctorProvider {
                        id: (*id).to_string(),
                        display_name,
                        on_disk_sessions: sessions.len(),
                        error: None,
                    },
                    Err(error) => DoctorProvider {
                        id: (*id).to_string(),
                        display_name,
                        on_disk_sessions: 0,
                        error: Some(format!("{error:#}")),
                    },
                },
                None => DoctorProvider {
                    id: (*id).to_string(),
                    display_name,
                    on_disk_sessions: 0,
                    error: Some("provider is not registered".to_string()),
                },
            }
        })
        .collect::<Vec<_>>();
    let report = DoctorReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        install_source,
        database,
        providers,
        orphan_artifact_manifests,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("memorph {}", report.version);
        println!(
            "Install source: {}",
            report.install_source.as_deref().unwrap_or("unknown")
        );
        println!("\nDatabase: {}", report.database.path);
        println!("  Size: {} bytes", report.database.size_bytes);
        println!("  Sessions: {}", report.database.sessions);
        println!("  Snapshots: {}", report.database.snapshots);
        println!("  Stale snapshots: {}", report.database.stale_snapshots);
        println!("\nProviders:");
        for provider in &report.providers {
            match &provider.error {
                Some(error) => println!(
                    "  {} ({}) | error: {}",
                    provider.display_name, provider.id, error
                ),
                None => println!(
                    "  {} ({}) | {} sessions",
                    provider.display_name, provider.id, provider.on_disk_sessions
                ),
            }
        }
        println!(
            "\nOrphan artifact manifests: {}",
            report.orphan_artifact_manifests
        );
        if report.database.snapshots == 0 {
            println!("\nNo projected snapshots. Run `memorph list` to bootstrap sessions.");
        }
    }
    Ok(())
}

fn print_session_list(
    all: bool,
    providers: Vec<String>,
    sort: memorph_cli::cli::ListSort,
    limit: Option<usize>,
    offset: usize,
    filter: core::SessionListFilter,
    json: bool,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let cwd_str = cwd.to_string_lossy().to_string();
    let sort = match sort {
        memorph_cli::cli::ListSort::Recent => core::SessionListSort::Recent,
        memorph_cli::cli::ListSort::Title => core::SessionListSort::Title,
    };
    let params = || core::SessionListParams {
        all,
        providers: providers.clone(),
        cwd: Some(cwd_str.clone()),
        fields: core::SessionListFields::WithStats,
        limit,
        offset: Some(offset),
        sort: sort.clone(),
        filter: filter.clone(),
    };
    let mut groups = core::projection::list_sessions(&params())?;
    if groups.iter().all(|group| group.sessions.is_empty())
        && providers.is_empty()
        && filter == core::SessionListFilter::default()
        && local_store::open_database()?.query_row(
            "SELECT COUNT(*) FROM session_snapshots",
            [],
            |row| row.get::<_, i64>(0),
        )? == 0
    {
        core::projection::bootstrap_session_projections(None, ActivityActor::Cli)?;
        groups = core::projection::list_sessions(&params())?;
    }
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
    fn parses_time_arguments() {
        assert_eq!(parse_time_arg("1970-01-01T00:00:01Z").unwrap(), 1_000);
        let date = NaiveDate::parse_from_str("2026-08-21", "%Y-%m-%d").unwrap();
        let expected = Local
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(parse_time_arg("2026-08-21").unwrap(), expected);
        let before = Utc::now().timestamp_millis() - 60 * 60 * 1_000;
        let parsed = parse_time_arg("1h").unwrap();
        let after = Utc::now().timestamp_millis() - 60 * 60 * 1_000;
        assert!((before..=after).contains(&parsed));
        assert!(parse_time_arg("bogus").is_err());
        assert!(parse_time_arg("-1h").is_err());
        assert!(parse_time_arg("天").is_err());
    }

    #[test]
    fn parses_size_arguments() {
        assert_eq!(parse_size_arg("42").unwrap(), 42);
        assert_eq!(parse_size_arg("1K").unwrap(), 1_024);
        assert_eq!(parse_size_arg("10M").unwrap(), 10 * 1_024 * 1_024);
        assert_eq!(parse_size_arg("1g").unwrap(), 1_024 * 1_024 * 1_024);
        assert!(parse_size_arg("1T").is_err());
        assert!(parse_size_arg("18446744073709551615G").is_err());
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
        assert!(output.contains("  provider_payload:"));
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
