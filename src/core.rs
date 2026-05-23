use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::canonical::{
    CanonicalSession, ImportedSession, SessionArtifact, SessionEvent, SessionEventKind,
};
use crate::format;
use crate::provider::ProviderSessionSummary;
use crate::storage::session_state::{self, SessionStateStore};
use crate::{provider, providers};

pub mod manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListParams {
    pub all: bool,
    pub providers: Vec<String>,
    pub cwd: Option<String>,
    pub include_message_counts: bool,
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
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_targets: Vec<String>,
    pub project_dir: Option<String>,
    pub last_active_at: Option<i64>,
    pub source_path: Option<String>,
    pub provider_id: String,
    pub message_count: Option<usize>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDetailView {
    pub provider_id: String,
    pub provider_name: String,
    pub session_id: String,
    pub canonical_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
    pub local_state: session_state::ResolvedLocalSessionState,
    pub event_count: usize,
    pub message_count: usize,
    pub artifact_count: usize,
    pub events: Vec<SessionEvent>,
    pub artifacts: Vec<SessionArtifact>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResolvedSessionState {
    native_title: Option<String>,
    local: session_state::ResolvedLocalSessionState,
}

impl ResolvedSessionState {
    fn resolved_title(&self) -> Option<&str> {
        self.local
            .display_title
            .as_deref()
            .or(self.native_title.as_deref())
    }
}

impl From<(&ProviderSessionSummary, &str)> for SessionItem {
    fn from((meta, provider_id): (&ProviderSessionSummary, &str)) -> Self {
        Self {
            session_id: meta.session_id.clone(),
            title: meta.title.clone(),
            native_title: meta.title.clone(),
            display_title: None,
            hidden: false,
            pinned: false,
            preferred_targets: Vec::new(),
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
    let explicit_provider_filter = !params.providers.is_empty();
    let session_states = session_state::load_state_store().unwrap_or_default();
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
        let Some(sessions) = scan_sessions_for_aggregate(prov.as_ref(), explicit_provider_filter)?
        else {
            continue;
        };
        let filtered_summaries: Vec<&ProviderSessionSummary> = if params.all {
            sessions.iter().collect()
        } else {
            let cwd = params.cwd.as_deref().unwrap_or("");
            sessions
                .iter()
                .filter(|s| s.project_dir.as_ref().map(|d| d == cwd).unwrap_or(false))
                .collect()
        };
        let session_ids: Vec<&str> = filtered_summaries
            .iter()
            .map(|s| s.session_id.as_str())
            .collect();
        let sizes = prov.session_sizes(&session_ids);
        let mut filtered: Vec<SessionItem> = filtered_summaries
            .iter()
            .map(|s| {
                enrich_session_item(
                    prov.as_ref(),
                    capabilities,
                    pid.as_str(),
                    s,
                    &session_states,
                    &sizes,
                    params.include_message_counts,
                )
            })
            .collect();
        filtered.sort_by_key(|s| {
            (
                std::cmp::Reverse(s.pinned),
                std::cmp::Reverse(s.last_active_at),
            )
        });

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

fn scan_sessions_for_aggregate(
    provider: &dyn provider::Provider,
    explicit_provider_filter: bool,
) -> Result<Option<Vec<ProviderSessionSummary>>> {
    match provider.scan_sessions() {
        Ok(sessions) => Ok(Some(sessions)),
        Err(err) if explicit_provider_filter => Err(err),
        Err(_) => Ok(None),
    }
}

fn enrich_session_item(
    provider: &dyn provider::Provider,
    capabilities: provider::ProviderCapabilities,
    provider_id: &str,
    meta: &ProviderSessionSummary,
    session_states: &SessionStateStore,
    sizes: &HashMap<String, u64>,
    include_message_count: bool,
) -> SessionItem {
    let mut item = SessionItem::from((meta, provider_id));
    let state = resolve_session_state(
        provider_id,
        &meta.session_id,
        meta.title.clone(),
        meta.project_dir.as_deref(),
        session_states,
    );
    apply_session_item_state(&mut item, &state);
    item.size_bytes = sizes.get(&meta.session_id).copied().or_else(|| {
        meta.source_path.as_deref().and_then(|path| {
            std::fs::metadata(path)
                .ok()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len())
        })
    });

    if include_message_count && capabilities.import {
        if let Some(source_path) = meta.source_path.as_deref() {
            item.message_count = provider
                .import_session(source_path)
                .ok()
                .map(|imported| session_message_count(&imported.session));
        }
    }

    item
}

pub fn get_canonical_session(provider_id: &str, session_id: &str) -> Result<ImportedSession> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let capabilities = prov.capabilities();
    if !capabilities.scan || !capabilities.import {
        anyhow::bail!(
            "Provider does not support loading sessions: {}",
            provider_id
        );
    }

    let meta = prov
        .scan_sessions()?
        .into_iter()
        .find(|session| session.session_id == session_id)
        .with_context(|| format!("Session not found: {}", session_id))?;

    load_canonical_session_from_meta(prov.as_ref(), provider_id, meta)
}

pub fn get_session_detail_view(provider_id: &str, session_id: &str) -> Result<SessionDetailView> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let capabilities = prov.capabilities();
    if !capabilities.scan || !capabilities.import {
        anyhow::bail!(
            "Provider does not support loading sessions: {}",
            provider_id
        );
    }

    let meta = prov
        .scan_sessions()?
        .into_iter()
        .find(|session| session.session_id == session_id)
        .with_context(|| format!("Session not found: {}", session_id))?;
    let source_path = meta.source_path.clone();
    let workspace_dir = meta.project_dir.clone();
    let native_title = meta.title.clone();
    let imported = load_canonical_session_from_meta(prov.as_ref(), provider_id, meta)?;
    let local_state =
        get_resolved_local_session_state(provider_id, session_id, workspace_dir.as_deref());

    Ok(build_session_detail_view(
        provider_id,
        prov.name(),
        session_id,
        source_path,
        prov.resume_command(session_id),
        native_title,
        local_state,
        imported,
    ))
}

pub fn get_resolved_local_session_state(
    provider_id: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> session_state::ResolvedLocalSessionState {
    let session_states = session_state::load_state_store().unwrap_or_default();
    session_state::resolve_session_state(&session_states, provider_id, session_id, workspace_dir)
}

fn build_session_detail_view(
    provider_id: &str,
    provider_name: &str,
    session_id: &str,
    source_path: Option<String>,
    resume_command: Option<String>,
    native_title: Option<String>,
    local_state: session_state::ResolvedLocalSessionState,
    imported: ImportedSession,
) -> SessionDetailView {
    let session = imported.session;
    let display_title = local_state.display_title.clone();
    let title = display_title
        .clone()
        .or_else(|| native_title.clone())
        .or_else(|| session.identity.source_title.clone());
    let message_count = session_message_count(&session);
    let source_path = source_path.or_else(|| session.provenance.primary_source.source_path.clone());

    SessionDetailView {
        provider_id: provider_id.to_string(),
        provider_name: provider_name.to_string(),
        session_id: session_id.to_string(),
        canonical_id: session.identity.canonical_id.clone(),
        title,
        native_title,
        display_title,
        workspace_dir: session.context.workspace_dir.clone(),
        created_at: session.context.created_at,
        last_active_at: session.context.last_active_at,
        source_path,
        resume_command,
        local_state,
        event_count: session.events.len(),
        message_count,
        artifact_count: session.artifacts.len(),
        events: session.events,
        artifacts: session.artifacts,
    }
}

fn resolve_session_state(
    provider_id: &str,
    session_id: &str,
    native_title: Option<String>,
    workspace_dir: Option<&str>,
    session_states: &SessionStateStore,
) -> ResolvedSessionState {
    ResolvedSessionState {
        native_title,
        local: session_state::resolve_session_state(
            session_states,
            provider_id,
            session_id,
            workspace_dir,
        ),
    }
}

fn apply_session_item_state(item: &mut SessionItem, state: &ResolvedSessionState) {
    item.native_title = state.native_title.clone();
    item.display_title = state.local.display_title.clone();
    item.title = state.resolved_title().map(str::to_string);
    item.hidden = state.local.hidden;
    item.pinned = state.local.pinned;
    item.preferred_targets = state.local.preferred_targets.clone();
}

fn session_message_count(session: &CanonicalSession) -> usize {
    session
        .events
        .iter()
        .filter(|event| !matches!(event.kind, SessionEventKind::Lifecycle))
        .count()
}

fn load_canonical_session_from_meta(
    provider: &dyn provider::Provider,
    provider_id: &str,
    meta: ProviderSessionSummary,
) -> Result<ImportedSession> {
    let source_path = meta
        .source_path
        .as_deref()
        .context("Session has no source path")?;
    let mut imported = provider.import_session(source_path)?;
    if imported.session.identity.source_title.is_none() {
        imported.session.identity.source_title = meta.title.clone();
    }
    if imported.session.context.workspace_dir.is_none() {
        imported.session.context.workspace_dir = meta.project_dir.clone();
    }
    if imported.session.context.last_active_at.is_none() {
        imported.session.context.last_active_at = meta
            .last_active_at
            .and_then(chrono::DateTime::from_timestamp_millis);
    }
    if imported
        .session
        .provenance
        .aliases
        .iter()
        .all(|alias| alias.provider_id != provider_id || alias.session_id != meta.session_id)
    {
        imported
            .session
            .provenance
            .aliases
            .push(crate::canonical::ProviderSessionRef {
                provider_id: provider_id.to_string(),
                session_id: meta.session_id,
                source_path: meta.source_path,
            });
    }
    Ok(imported)
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
    let imported = get_canonical_session(&params.provider, &params.session_id)?;

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
        format::write_session(&path, &imported.session)?;
        files.push(path.display().to_string());
    }
    if write_json {
        let path = PathBuf::from(format!("{}.json", prefix));
        let json = serde_json::to_string_pretty(&imported.session)?;
        std::fs::write(&path, json)?;
        files.push(path.display().to_string());
    }
    if write_markdown {
        let path = PathBuf::from(format!("{}.md", prefix));
        format::write_markdown(&path, &imported.session)?;
        files.push(path.display().to_string());
    }
    if write_html {
        let path = PathBuf::from(format!("{}.html", prefix));
        format::write_html(&path, &imported.session)?;
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
        get_canonical_session(&params.provider, &params.file_or_id)?.session
    };

    let target_prov = providers::find_provider(&params.provider)
        .with_context(|| format!("Target provider not available: {}", params.provider))?;
    let target_capabilities = target_prov.capabilities();
    if !target_capabilities.export {
        anyhow::bail!(
            "Provider does not support writing sessions: {}",
            params.provider
        );
    }
    let exported = target_prov.export_session(&session, &target_dir)?;

    Ok(ImportResult {
        provider_name: target_prov.name().to_string(),
        new_session_id: exported.session_id,
        resume_command: exported.resume_command,
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
    session_state::remove_session(provider_id, session_id)?;
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

    session_state::set_display_title(provider_id, session_id, new_title)?;
    Ok(RenameResult {
        provider_name: prov.name().to_string(),
        session_id: session_id.to_string(),
        display_title: new_title.to_string(),
        native_updated,
        warning,
    })
}

pub fn update_session_local_state(
    provider_id: &str,
    session_id: &str,
    update: &session_state::SessionLocalStateUpdate,
) -> Result<session_state::ResolvedLocalSessionState> {
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

    session_state::update_session_state(provider_id, session_id, update)
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
    if !source_capabilities.scan || !source_capabilities.import {
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

    let source_session_id = session_meta.session_id.clone();
    let imported =
        load_canonical_session_from_meta(source_prov.as_ref(), &params.from, session_meta)?;

    let target_prov = providers::find_provider(&params.to)
        .with_context(|| format!("Unknown target provider: {}", params.to))?;
    let target_capabilities = target_prov.capabilities();
    if !target_capabilities.export {
        anyhow::bail!(
            "Target provider does not support writing sessions: {}",
            params.to
        );
    }
    let exported = target_prov.export_session(&imported.session, &target_dir)?;

    Ok(SwitchResult {
        from_name: source_prov.name().to_string(),
        to_name: target_prov.name().to_string(),
        source_session_id,
        target_session_id: exported.session_id,
        resume_command: exported.resume_command,
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
    let explicit_provider_filter = !params.providers.is_empty();
    let session_states = session_state::load_state_store().unwrap_or_default();
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
        let Some(sessions) = scan_sessions_for_aggregate(prov.as_ref(), explicit_provider_filter)?
        else {
            continue;
        };
        let filtered: Vec<SessionItem> = sessions
            .iter()
            .map(|s| {
                let mut item = SessionItem::from((s, pid.as_str()));
                let state = resolve_session_state(
                    pid,
                    &s.session_id,
                    s.title.clone(),
                    s.project_dir.as_deref(),
                    &session_states,
                );
                apply_session_item_state(&mut item, &state);
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
    use crate::canonical::{
        CanonicalSchema, EventBlock, EventLinks, EventMetadata, EventRole, EventSource,
        MappingDirection, MappingDisposition, MappingReport, ProviderSessionRef, SessionContext,
        SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance,
    };
    use crate::storage::session_state::SessionStateStore;
    use chrono::Utc;
    use std::collections::BTreeMap;

    struct FailingProvider;

    impl provider::Provider for FailingProvider {
        fn id(&self) -> &'static str {
            "failing"
        }

        fn name(&self) -> &'static str {
            "Failing"
        }

        fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
            anyhow::bail!("scan failed")
        }

        fn import_session(&self, _source_path: &str) -> Result<ImportedSession> {
            anyhow::bail!("unused")
        }
    }

    #[test]
    fn aggregate_scan_skips_provider_error_without_explicit_filter() {
        let sessions = scan_sessions_for_aggregate(&FailingProvider, false).unwrap();

        assert!(sessions.is_none());
    }

    #[test]
    fn aggregate_scan_keeps_provider_error_with_explicit_filter() {
        let error = scan_sessions_for_aggregate(&FailingProvider, true).unwrap_err();

        assert!(error.to_string().contains("scan failed"));
    }

    #[test]
    fn session_item_overlay_prefers_memorph_display_title() {
        let meta = ProviderSessionSummary {
            session_id: "session-1".to_string(),
            title: Some("Native".to_string()),
            project_dir: Some("/tmp/project".to_string()),
            last_active_at: Some(42),
            source_path: Some("/tmp/session.jsonl".to_string()),
        };
        let mut item = SessionItem::from((&meta, "codex"));
        let mut session_states = SessionStateStore::default();
        session_state::set_display_title_in_store(
            &mut session_states,
            "codex",
            "session-1",
            "Display",
        );

        let state = resolve_session_state(
            "codex",
            "session-1",
            meta.title.clone(),
            meta.project_dir.as_deref(),
            &session_states,
        );
        apply_session_item_state(&mut item, &state);

        assert_eq!(item.native_title.as_deref(), Some("Native"));
        assert_eq!(item.display_title.as_deref(), Some("Display"));
        assert_eq!(item.title.as_deref(), Some("Display"));
    }

    #[test]
    fn session_detail_view_prefers_local_display_title_and_counts_non_lifecycle_messages() {
        let imported = ImportedSession {
            session: crate::canonical::CanonicalSession {
                schema: CanonicalSchema::default(),
                identity: SessionIdentity {
                    canonical_id: "canonical-1".to_string(),
                    source_title: Some("Native".to_string()),
                },
                provenance: SessionProvenance {
                    imported_at: Utc::now(),
                    imported_by: Some("test".to_string()),
                    primary_source: ProviderSessionRef {
                        provider_id: "codex".to_string(),
                        session_id: "session-1".to_string(),
                        source_path: Some("/tmp/session.jsonl".to_string()),
                    },
                    aliases: Vec::new(),
                },
                context: SessionContext {
                    workspace_dir: Some("/tmp/project".to_string()),
                    created_at: Some(Utc::now()),
                    last_active_at: Some(Utc::now()),
                    tags: Vec::new(),
                },
                events: vec![
                    SessionEvent {
                        id: "e1".to_string(),
                        kind: SessionEventKind::Lifecycle,
                        role: EventRole::System,
                        timestamp: Utc::now(),
                        links: EventLinks::default(),
                        blocks: vec![EventBlock::Text {
                            text: "started".to_string(),
                        }],
                        metadata: EventMetadata {
                            source: EventSource {
                                provider_id: "codex".to_string(),
                                original_id: None,
                                original_role: None,
                                phase: None,
                            },
                            model: None,
                            usage: None,
                            fidelity: MappingDisposition::Preserved,
                            provider_ext: BTreeMap::new(),
                        },
                    },
                    SessionEvent {
                        id: "e2".to_string(),
                        kind: SessionEventKind::Message,
                        role: EventRole::Assistant,
                        timestamp: Utc::now(),
                        links: EventLinks::default(),
                        blocks: vec![EventBlock::Text {
                            text: "hello".to_string(),
                        }],
                        metadata: EventMetadata {
                            source: EventSource {
                                provider_id: "codex".to_string(),
                                original_id: None,
                                original_role: None,
                                phase: None,
                            },
                            model: None,
                            usage: None,
                            fidelity: MappingDisposition::Preserved,
                            provider_ext: BTreeMap::new(),
                        },
                    },
                ],
                artifacts: Vec::new(),
                extensions: BTreeMap::new(),
            },
            report: MappingReport::new("codex", MappingDirection::Import),
        };

        let view = build_session_detail_view(
            "codex",
            "codex",
            "session-1",
            Some("/tmp/session.jsonl".to_string()),
            Some("codex resume session-1".to_string()),
            Some("Native".to_string()),
            session_state::ResolvedLocalSessionState {
                display_title: Some("Display".to_string()),
                archived: false,
                hidden: false,
                pinned: false,
                notes: None,
                tags: Vec::new(),
                preferred_targets: Vec::new(),
            },
            imported,
        );

        assert_eq!(view.title.as_deref(), Some("Display"));
        assert_eq!(view.native_title.as_deref(), Some("Native"));
        assert_eq!(view.display_title.as_deref(), Some("Display"));
        assert_eq!(view.event_count, 2);
        assert_eq!(view.message_count, 1);
        assert_eq!(view.source_path.as_deref(), Some("/tmp/session.jsonl"));
    }
}
