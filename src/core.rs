use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::format;
use crate::model::{MemorphSession, SessionMeta};
use crate::providers;

pub mod manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListParams {
    pub all: bool,
    pub providers: Vec<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGroup {
    pub provider_id: String,
    pub provider_name: String,
    pub sessions: Vec<SessionItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionItem {
    pub session_id: String,
    pub title: Option<String>,
    pub project_dir: Option<String>,
    pub last_active_at: Option<i64>,
    pub source_path: Option<String>,
    pub provider_id: String,
}

impl From<(&SessionMeta, &str)> for SessionItem {
    fn from((meta, provider_id): (&SessionMeta, &str)) -> Self {
        Self {
            session_id: meta.session_id.clone(),
            title: meta.title.clone(),
            project_dir: meta.project_dir.clone(),
            last_active_at: meta.last_active_at,
            source_path: meta.source_path.clone(),
            provider_id: provider_id.to_string(),
        }
    }
}

pub fn resolve_providers(filter: &[String]) -> Vec<String> {
    if filter.is_empty() {
        providers::all_provider_ids()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        filter.to_vec()
    }
}

pub fn list_sessions(params: &SessionListParams) -> Result<Vec<SessionGroup>> {
    let provider_ids = resolve_providers(&params.providers);
    let mut groups = Vec::new();

    for pid in &provider_ids {
        let prov = match providers::find_provider(pid) {
            Some(p) => p,
            None => continue,
        };
        let capabilities = prov.capabilities();
        if !capabilities.scan {
            continue;
        }
        let sessions = prov.scan_sessions()?;
        let mut filtered: Vec<SessionItem> = if params.all {
            sessions
                .iter()
                .map(|s| SessionItem::from((s, pid.as_str())))
                .collect()
        } else {
            let cwd = params.cwd.as_deref().unwrap_or("");
            sessions
                .iter()
                .filter(|s| s.project_dir.as_ref().map(|d| d == cwd).unwrap_or(false))
                .map(|s| SessionItem::from((s, pid.as_str())))
                .collect()
        };
        filtered.sort_by_key(|s| std::cmp::Reverse(s.last_active_at));

        if !filtered.is_empty() {
            groups.push(SessionGroup {
                provider_id: pid.clone(),
                provider_name: prov.name().to_string(),
                sessions: filtered,
            });
        }
    }

    Ok(groups)
}

pub fn get_session(provider_id: &str, session_id: &str) -> Result<MemorphSession> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let capabilities = prov.capabilities();
    if !capabilities.scan || !capabilities.load {
        anyhow::bail!(
            "Provider does not support loading sessions: {}",
            provider_id
        );
    }
    let sessions = prov.scan_sessions()?;
    let meta = sessions
        .into_iter()
        .find(|s| s.session_id == session_id)
        .with_context(|| format!("Session not found: {}", session_id))?;
    let source_path = meta
        .source_path
        .as_deref()
        .context("Session has no source path")?;
    let mut session = prov.load_session(source_path)?;
    if session.session.title.is_none() {
        session.session.title = meta.title.clone();
    }
    Ok(session)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportParams {
    pub provider: String,
    pub session_id: String,
    pub output_prefix: Option<String>,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub files: Vec<String>,
}

pub fn export_session(params: &ExportParams) -> Result<ExportResult> {
    let mut session = get_session(&params.provider, &params.session_id)?;
    session.meta.source_session_id = params.session_id.clone();
    session.meta.source_provider = params.provider.clone();

    let prefix = params
        .output_prefix
        .as_deref()
        .unwrap_or(&params.session_id);
    let mut files = Vec::new();

    let write_morph = params.format == "morph" || params.format == "both";
    let write_json = params.format == "json" || params.format == "both";
    let write_markdown = params.format == "md" || params.format == "markdown";
    let write_html = params.format == "html";

    if !write_morph && !write_json && !write_markdown && !write_html {
        anyhow::bail!(
            "Unsupported format: {}. Use 'json', 'md', 'html', 'morph', or 'both'",
            params.format
        );
    }

    if write_morph {
        let path = PathBuf::from(format!("{}.morph", prefix));
        format::write_session(&path, &session)?;
        files.push(path.display().to_string());
    }
    if write_json {
        let path = PathBuf::from(format!("{}.json", prefix));
        let json = serde_json::to_string_pretty(&session)?;
        std::fs::write(&path, json)?;
        files.push(path.display().to_string());
    }
    if write_markdown {
        let path = PathBuf::from(format!("{}.md", prefix));
        format::write_markdown(&path, &session)?;
        files.push(path.display().to_string());
    }
    if write_html {
        let path = PathBuf::from(format!("{}.html", prefix));
        format::write_html(&path, &session)?;
        files.push(path.display().to_string());
    }

    Ok(ExportResult { files })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportParams {
    pub provider: String,
    pub file_or_id: String,
    pub to_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub provider_name: String,
    pub new_session_id: String,
    pub resume_command: Option<String>,
}

pub fn import_session(params: &ImportParams) -> Result<ImportResult> {
    let cwd = std::env::current_dir()?;
    let target_dir = if let Some(dir) = &params.to_dir {
        let p = Path::new(dir);
        if !p.exists() {
            anyhow::bail!("Target directory does not exist: {}", dir);
        }
        p.canonicalize()?
    } else {
        cwd
    };

    let session = if params.file_or_id.ends_with(".morph")
        || params.file_or_id.ends_with(".json")
        || params.file_or_id.ends_with(".md")
        || params.file_or_id.ends_with(".html")
    {
        let path = Path::new(&params.file_or_id);
        if params.file_or_id.ends_with(".morph") {
            format::read_session(path)?
        } else if params.file_or_id.ends_with(".json") {
            let json = std::fs::read_to_string(path)?;
            serde_json::from_str(&json)?
        } else if params.file_or_id.ends_with(".md") {
            format::read_markdown(path)?
        } else {
            format::read_html(path)?
        }
    } else {
        get_session(&params.provider, &params.file_or_id)?
    };

    let target_prov = providers::find_provider(&params.provider)
        .with_context(|| format!("Target provider not available: {}", params.provider))?;
    let target_capabilities = target_prov.capabilities();
    if !target_capabilities.write {
        anyhow::bail!(
            "Provider does not support writing sessions: {}",
            params.provider
        );
    }
    let new_id = target_prov.write_session(&session, &target_dir)?;
    let resume = if target_capabilities.resume {
        target_prov.resume_command(&new_id)
    } else {
        None
    };

    Ok(ImportResult {
        provider_name: target_prov.name().to_string(),
        new_session_id: new_id,
        resume_command: resume,
    })
}

pub fn delete_session(provider_id: &str, session_id: &str) -> Result<()> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    if !prov.capabilities().delete {
        anyhow::bail!(
            "Provider does not support deleting sessions: {}",
            provider_id
        );
    }
    prov.delete_session(session_id)?;
    Ok(())
}

pub fn rename_session(provider_id: &str, session_id: &str, new_title: &str) -> Result<()> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    if !prov.capabilities().rename {
        anyhow::bail!(
            "Provider does not support renaming sessions: {}",
            provider_id
        );
    }
    prov.rename_session(session_id, new_title)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchParams {
    pub from: String,
    pub to: String,
    pub session_id: Option<String>,
    pub to_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchResult {
    pub from_name: String,
    pub to_name: String,
    pub source_session_id: String,
    pub target_session_id: String,
    pub resume_command: Option<String>,
}

pub fn switch_session(params: &SwitchParams) -> Result<SwitchResult> {
    let cwd = std::env::current_dir()?;
    let target_dir = if let Some(dir) = &params.to_dir {
        let p = Path::new(dir);
        if !p.exists() {
            anyhow::bail!("Target directory does not exist: {}", dir);
        }
        p.canonicalize()?
    } else {
        cwd.clone()
    };

    let source_prov = providers::find_provider(&params.from)
        .with_context(|| format!("Unknown source provider: {}", params.from))?;
    let source_capabilities = source_prov.capabilities();
    if !source_capabilities.scan || !source_capabilities.load {
        anyhow::bail!(
            "Source provider does not support reading sessions: {}",
            params.from
        );
    }
    let sessions = source_prov.scan_sessions()?;
    let cwd_str = cwd.to_string_lossy().to_string();

    let session_meta = if let Some(id) = &params.session_id {
        sessions
            .into_iter()
            .find(|s| s.session_id == *id)
            .with_context(|| format!("Session not found: {}", id))?
    } else {
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
        candidates.into_iter().next().with_context(|| {
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
    session.meta.source_provider = params.from.clone();
    if session.session.title.is_none() {
        session.session.title = session_meta.title.clone();
    }

    let target_prov = providers::find_provider(&params.to)
        .with_context(|| format!("Unknown target provider: {}", params.to))?;
    let target_capabilities = target_prov.capabilities();
    if !target_capabilities.write {
        anyhow::bail!(
            "Target provider does not support writing sessions: {}",
            params.to
        );
    }
    let new_id = target_prov.write_session(&session, &target_dir)?;
    let resume = if target_capabilities.resume {
        target_prov.resume_command(&new_id)
    } else {
        None
    };

    Ok(SwitchResult {
        from_name: source_prov.name().to_string(),
        to_name: target_prov.name().to_string(),
        source_session_id: session_meta.session_id,
        target_session_id: new_id,
        resume_command: resume,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindParams {
    pub dir: Option<String>,
    pub session: Option<String>,
    pub providers: Vec<String>,
}

pub fn find_sessions(params: &FindParams) -> Result<Vec<SessionGroup>> {
    let provider_ids = resolve_providers(&params.providers);
    let mut groups = Vec::new();

    for pid in &provider_ids {
        let prov = match providers::find_provider(pid) {
            Some(p) => p,
            None => continue,
        };
        let capabilities = prov.capabilities();
        if !capabilities.scan {
            continue;
        }
        let sessions = prov.scan_sessions()?;
        let filtered: Vec<SessionItem> = sessions
            .iter()
            .filter(|s| {
                let dir_match = params.dir.as_ref().map_or(true, |d| {
                    s.project_dir
                        .as_ref()
                        .map(|pd| pd.contains(d.as_str()))
                        .unwrap_or(false)
                });
                let session_match = params.session.as_ref().map_or(true, |pat| {
                    s.session_id.contains(pat.as_str())
                        || s.title
                            .as_ref()
                            .map(|t| t.contains(pat.as_str()))
                            .unwrap_or(false)
                });
                dir_match && session_match
            })
            .map(|s| SessionItem::from((s, pid.as_str())))
            .collect();

        if !filtered.is_empty() {
            groups.push(SessionGroup {
                provider_id: pid.clone(),
                provider_name: prov.name().to_string(),
                sessions: filtered,
            });
        }
    }

    Ok(groups)
}
