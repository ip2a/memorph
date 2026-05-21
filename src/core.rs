use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::format;
use crate::model::{MemorphSession, SessionMeta};
use crate::providers;
use crate::storage::session_overrides::{self, SessionOverrides};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    pub project_dir: Option<String>,
    pub last_active_at: Option<i64>,
    pub source_path: Option<String>,
    pub provider_id: String,
    pub message_count: Option<usize>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResolvedSessionTitles {
    native_title: Option<String>,
    display_title: Option<String>,
}

impl ResolvedSessionTitles {
    fn resolved_title(&self) -> Option<&str> {
        self.display_title
            .as_deref()
            .or(self.native_title.as_deref())
    }
}

#[derive(Debug, Clone)]
struct LoadedSession {
    meta: SessionMeta,
    session: MemorphSession,
}

impl From<(&SessionMeta, &str)> for SessionItem {
    fn from((meta, provider_id): (&SessionMeta, &str)) -> Self {
        Self {
            session_id: meta.session_id.clone(),
            title: meta.title.clone(),
            native_title: meta.title.clone(),
            display_title: None,
            project_dir: meta.project_dir.clone(),
            last_active_at: meta.last_active_at,
            source_path: meta.source_path.clone(),
            provider_id: provider_id.to_string(),
            message_count: None,
            size_bytes: None,
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
    let overrides = session_overrides::load_overrides().unwrap_or_default();
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
                .map(|s| {
                    enrich_session_item(prov.as_ref(), capabilities, pid.as_str(), s, &overrides)
                })
                .collect()
        } else {
            let cwd = params.cwd.as_deref().unwrap_or("");
            sessions
                .iter()
                .filter(|s| s.project_dir.as_ref().map(|d| d == cwd).unwrap_or(false))
                .map(|s| {
                    enrich_session_item(prov.as_ref(), capabilities, pid.as_str(), s, &overrides)
                })
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

fn enrich_session_item(
    provider: &dyn crate::provider::Provider,
    capabilities: crate::provider::ProviderCapabilities,
    provider_id: &str,
    meta: &SessionMeta,
    overrides: &SessionOverrides,
) -> SessionItem {
    let mut item = SessionItem::from((meta, provider_id));
    let titles =
        resolve_session_titles(provider_id, &meta.session_id, meta.title.clone(), overrides);
    apply_session_item_titles(&mut item, &titles);
    item.size_bytes = provider
        .session_size(&meta.session_id)
        .ok()
        .filter(|size| *size > 0)
        .or_else(|| {
            meta.source_path.as_deref().and_then(|path| {
                std::fs::metadata(path)
                    .ok()
                    .filter(|metadata| metadata.is_file())
                    .map(|metadata| metadata.len())
            })
        });

    if capabilities.load {
        if let Some(source_path) = meta.source_path.as_deref() {
            item.message_count = provider
                .load_session(source_path)
                .ok()
                .map(|session| session.messages.len());
        }
    }

    item
}

pub fn get_session(provider_id: &str, session_id: &str) -> Result<MemorphSession> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let overrides = session_overrides::load_overrides().unwrap_or_default();
    Ok(load_resolved_session(prov.as_ref(), provider_id, session_id, &overrides)?.session)
}

fn resolve_session_titles(
    provider_id: &str,
    session_id: &str,
    native_title: Option<String>,
    overrides: &SessionOverrides,
) -> ResolvedSessionTitles {
    ResolvedSessionTitles {
        native_title,
        display_title: session_overrides::get_display_title(overrides, provider_id, session_id)
            .map(str::to_string),
    }
}

fn apply_session_item_titles(item: &mut SessionItem, titles: &ResolvedSessionTitles) {
    item.native_title = titles.native_title.clone();
    item.display_title = titles.display_title.clone();
    item.title = titles.resolved_title().map(str::to_string);
}

fn apply_session_titles(session: &mut MemorphSession, titles: &ResolvedSessionTitles) {
    session.session.title = titles.resolved_title().map(str::to_string);
}

fn load_resolved_session(
    provider: &dyn crate::provider::Provider,
    provider_id: &str,
    session_id: &str,
    overrides: &SessionOverrides,
) -> Result<LoadedSession> {
    let capabilities = provider.capabilities();
    if !capabilities.scan || !capabilities.load {
        anyhow::bail!(
            "Provider does not support loading sessions: {}",
            provider_id
        );
    }

    let meta = provider
        .scan_sessions()?
        .into_iter()
        .find(|session| session.session_id == session_id)
        .with_context(|| format!("Session not found: {}", session_id))?;

    load_resolved_session_from_meta(provider, provider_id, meta, overrides)
}

fn load_resolved_session_from_meta(
    provider: &dyn crate::provider::Provider,
    provider_id: &str,
    meta: SessionMeta,
    overrides: &SessionOverrides,
) -> Result<LoadedSession> {
    let source_path = meta
        .source_path
        .as_deref()
        .context("Session has no source path")?;
    let mut session = provider.load_session(source_path)?;
    let titles = resolve_session_titles(
        provider_id,
        &meta.session_id,
        session.session.title.clone().or(meta.title.clone()),
        overrides,
    );
    apply_session_titles(&mut session, &titles);

    Ok(LoadedSession { meta, session })
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
    session_overrides::remove_session(provider_id, session_id)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameResult {
    pub provider_name: String,
    pub session_id: String,
    pub display_title: String,
    pub native_updated: bool,
    pub warning: Option<String>,
}

pub fn rename_session(
    provider_id: &str,
    session_id: &str,
    new_title: &str,
) -> Result<RenameResult> {
    let new_title = new_title.trim();
    if new_title.is_empty() {
        anyhow::bail!("Session title cannot be empty");
    }

    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let capabilities = prov.capabilities();
    if capabilities.scan {
        let exists = prov
            .scan_sessions()?
            .into_iter()
            .any(|session| session.session_id == session_id);
        if !exists {
            anyhow::bail!("Session not found: {}", session_id);
        }
    }

    let mut warning = None;
    let native_updated = if capabilities.rename {
        match prov.rename_session(session_id, new_title) {
            Ok(()) => true,
            Err(e) => {
                warning = Some(format!(
                    "Provider rename failed; memorph display title was saved: {e}"
                ));
                false
            }
        }
    } else {
        warning = Some(format!(
            "Provider does not support native rename; memorph display title was saved: {}",
            provider_id
        ));
        false
    };

    session_overrides::set_display_title(provider_id, session_id, new_title)?;
    Ok(RenameResult {
        provider_name: prov.name().to_string(),
        session_id: session_id.to_string(),
        display_title: new_title.to_string(),
        native_updated,
        warning,
    })
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

    let overrides = session_overrides::load_overrides().unwrap_or_default();
    let loaded = load_resolved_session_from_meta(
        source_prov.as_ref(),
        &params.from,
        session_meta,
        &overrides,
    )?;
    let mut session = loaded.session;
    session.meta.source_session_id = loaded.meta.session_id.clone();
    session.meta.source_provider = params.from.clone();

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
        source_session_id: loaded.meta.session_id,
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
    let overrides = session_overrides::load_overrides().unwrap_or_default();
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
            .map(|s| {
                let mut item = SessionItem::from((s, pid.as_str()));
                let titles =
                    resolve_session_titles(pid, &s.session_id, s.title.clone(), &overrides);
                apply_session_item_titles(&mut item, &titles);
                item
            })
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
                        || s.native_title
                            .as_ref()
                            .map(|t| t.contains(pat.as_str()))
                            .unwrap_or(false)
                });
                dir_match && session_match
            })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MemorphMeta, SessionInfo};

    #[test]
    fn session_item_overlay_prefers_memorph_display_title() {
        let meta = SessionMeta {
            session_id: "session-1".to_string(),
            title: Some("Native".to_string()),
            project_dir: Some("/tmp/project".to_string()),
            last_active_at: Some(42),
            source_path: Some("/tmp/session.jsonl".to_string()),
        };
        let mut item = SessionItem::from((&meta, "codex"));
        let mut overrides = SessionOverrides::default();
        session_overrides::set_display_title_in_overrides(
            &mut overrides,
            "codex",
            "session-1",
            "Display",
        );

        let titles = resolve_session_titles("codex", "session-1", meta.title.clone(), &overrides);
        apply_session_item_titles(&mut item, &titles);

        assert_eq!(item.native_title.as_deref(), Some("Native"));
        assert_eq!(item.display_title.as_deref(), Some("Display"));
        assert_eq!(item.title.as_deref(), Some("Display"));
    }

    #[test]
    fn full_session_overlay_prefers_memorph_display_title() {
        let mut session = MemorphSession {
            meta: MemorphMeta::default(),
            session: SessionInfo {
                id: "session-1".to_string(),
                title: Some("Native".to_string()),
                project_dir: None,
                created_at: None,
                last_active_at: None,
                tags: None,
            },
            messages: Vec::new(),
        };
        let mut overrides = SessionOverrides::default();
        session_overrides::set_display_title_in_overrides(
            &mut overrides,
            "codex",
            "session-1",
            "Display",
        );

        let titles = resolve_session_titles(
            "codex",
            "session-1",
            session.session.title.clone(),
            &overrides,
        );
        apply_session_titles(&mut session, &titles);

        assert_eq!(session.session.title.as_deref(), Some("Display"));
    }
}
