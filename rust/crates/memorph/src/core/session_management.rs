use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::canonical::{
    CanonicalSchema, CanonicalSession, ProviderSessionRef, SessionContext, SessionIdentity,
    SessionProvenance,
};
use crate::core::compression;
use crate::format;
use crate::provider;
use crate::providers;
use crate::storage::session_state;

use super::{
    ExpandCompressionSessionParams, ExportResult, RenameResult, RestoreCompressionArchiveParams,
};

pub fn normalized_workspace_key(provider_id: &str, workspace: Option<&str>) -> Option<String> {
    providers::find_provider(provider_id)
        .map(|provider| provider.normalized_workspace_key(workspace))
        .unwrap_or_else(|| provider::default_normalized_workspace_key(workspace))
}

pub fn workspace_matches(
    provider_id: &str,
    session_workspace: Option<&str>,
    requested_workspace: Option<&str>,
) -> bool {
    providers::find_provider(provider_id)
        .map(|provider| provider.workspace_matches(session_workspace, requested_workspace))
        .unwrap_or_else(|| {
            provider::default_workspace_matches(session_workspace, requested_workspace)
        })
}

pub fn resolve_existing_target_dir(provider_id: &str, input: Option<&str>) -> Result<PathBuf> {
    let provider = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    provider.resolve_workspace_dir(input)
}

pub fn delete_session(provider_id: &str, session_id: &str) -> Result<()> {
    delete_sessions(provider_id, &[session_id])
        .into_iter()
        .next()
        .unwrap_or_else(|| {
            Err(anyhow::anyhow!(
                "No delete result for session {}",
                session_id
            ))
        })
}

pub fn delete_sessions(provider_id: &str, session_ids: &[&str]) -> Vec<Result<()>> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id));
    let prov = match prov {
        Ok(provider) => provider,
        Err(err) => {
            let message = err.to_string();
            return session_ids
                .iter()
                .map(|session_id| {
                    Err(anyhow::anyhow!(
                        "Failed to delete session {}: {}",
                        session_id,
                        message
                    ))
                })
                .collect();
        }
    };
    if !prov.capabilities().delete {
        return session_ids
            .iter()
            .map(|session_id| {
                Err(anyhow::anyhow!(
                    "Provider does not support deleting sessions: {} ({})",
                    provider_id,
                    session_id
                ))
            })
            .collect();
    }
    prov.delete_sessions(session_ids)
        .into_iter()
        .zip(session_ids.iter())
        .map(|(result, session_id)| match result {
            Ok(()) => session_state::remove_session(provider_id, session_id),
            Err(err) => Err(err),
        })
        .collect()
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
        let cache = crate::cache::global_cache();
        let exists = cache
            .get_or_refresh(provider_id, || prov.scan_sessions())?
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
            Err(err) => {
                warning = Some(format!(
                    "Provider rename failed; memorph display title was saved: {err}"
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

pub fn prepare_session_for_export(
    session: &CanonicalSession,
    source_provider_id: &str,
    target_provider_id: &str,
) -> Result<(CanonicalSession, compression::CompressionReport)> {
    let policy = compression::CompressionPolicy::preserve(source_provider_id, target_provider_id);
    compression::prepare_for_export_with_archive(session, &policy)
}

pub fn prepare_session_for_target_provider(
    session: &CanonicalSession,
    target_provider_id: &str,
) -> Result<(CanonicalSession, compression::CompressionReport)> {
    let source_provider_id = session.provenance.primary_source.provider_id.trim();
    let source_provider_id = if source_provider_id.is_empty() {
        target_provider_id
    } else {
        source_provider_id
    };
    prepare_session_for_export(session, source_provider_id, target_provider_id)
}

pub fn expand_compression_session(params: &ExpandCompressionSessionParams) -> Result<ExportResult> {
    let session = read_session_export_file(&params.file)?;
    let source_provider_id = session.provenance.primary_source.provider_id.trim();
    let source_provider_id = if source_provider_id.is_empty() {
        "memorph"
    } else {
        source_provider_id
    };
    let policy = compression::CompressionPolicy::expand(source_provider_id, source_provider_id);
    let (expanded, _) = compression::prepare_for_export_with_archive(&session, &policy)?;
    let default_prefix = Path::new(&params.file)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| format!("{}_expanded", value))
        .unwrap_or_else(|| format!("{}_expanded", session.identity.canonical_id));
    let prefix = params.output_prefix.as_deref().unwrap_or(&default_prefix);
    write_session_export_files(&expanded, prefix, &params.format, None)
}

pub fn restore_compression_archive(
    params: &RestoreCompressionArchiveParams,
) -> Result<ExportResult> {
    let archive = compression::load_archive(&params.archive_ref)?;
    let session = session_from_compression_archive_for_tests(&params.archive_ref, archive)?;
    let default_prefix = format!("{}_compression_archive", session.identity.canonical_id);
    let prefix = params.output_prefix.as_deref().unwrap_or(&default_prefix);
    write_session_export_files(&session, prefix, &params.format, None)
}

pub fn list_compression_archives(
    workspace: Option<&str>,
) -> Result<Vec<compression::CompressionArchiveSummary>> {
    compression::list_archives_for_workspace(workspace)
}

pub fn list_compression_provider_support() -> Vec<crate::provider::ProviderCompressionSupport> {
    providers::all_provider_ids()
        .iter()
        .filter_map(|provider_id| {
            let provider = providers::find_provider(provider_id)?;
            let default_projection = provider.compression_projection();
            Some(crate::provider::ProviderCompressionSupport {
                provider_id: (*provider_id).to_string(),
                detects_native_source: provider.detects_native_compression_source(),
                native_target_projection: default_projection
                    == crate::provider::CompressionProjection::Native,
                default_projection,
            })
        })
        .collect()
}

pub fn read_session_export_file(file: &str) -> Result<CanonicalSession> {
    let path = Path::new(file);
    if file.ends_with(".morph") {
        format::read_session(path)
    } else if file.ends_with(".json") {
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json).with_context(|| format!("Failed to parse JSON: {}", file))
    } else if file.ends_with(".md") {
        format::read_markdown(path)
    } else if file.ends_with(".html") {
        format::read_html(path)
    } else {
        anyhow::bail!(
            "Unsupported session file: {}. Use .json, .md, .html, or .morph",
            file
        );
    }
}

pub fn write_session_export_files(
    session: &CanonicalSession,
    prefix: &str,
    format_name: &str,
    output_dir: Option<&Path>,
) -> Result<ExportResult> {
    let mut files = Vec::new();

    let write_morph = format_name == "morph" || format_name == "both";
    let write_json = format_name == "json" || format_name == "both";
    let write_markdown = format_name == "md" || format_name == "markdown";
    let write_html = format_name == "html";

    if !write_morph && !write_json && !write_markdown && !write_html {
        anyhow::bail!(
            "Unsupported format: {}. Use 'json', 'md', 'html', 'morph', or 'both'",
            format_name
        );
    }

    if let Some(dir) = output_dir {
        if !dir.exists() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Failed to create output directory: {}", dir.display()))?;
        }
    }

    let base = output_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(PathBuf::new);

    if write_morph {
        let path = base.join(format!("{}.morph", prefix));
        format::write_session(&path, session)?;
        files.push(path.display().to_string());
    }
    if write_json {
        let path = base.join(format!("{}.json", prefix));
        let json = serde_json::to_string_pretty(session)?;
        std::fs::write(&path, json)?;
        files.push(path.display().to_string());
    }
    if write_markdown {
        let path = base.join(format!("{}.md", prefix));
        format::write_markdown(&path, session)?;
        files.push(path.display().to_string());
    }
    if write_html {
        let path = base.join(format!("{}.html", prefix));
        format::write_html(&path, session)?;
        files.push(path.display().to_string());
    }

    Ok(ExportResult { files })
}

pub(crate) fn session_from_compression_archive_for_tests(
    archive_ref: &str,
    archive: compression::CompressionArchive,
) -> Result<CanonicalSession> {
    let created_at = archive.events.first().map(|event| event.timestamp);
    let last_active_at = archive.events.last().map(|event| event.timestamp);
    let archive_value = serde_json::to_value(&archive)?;

    Ok(CanonicalSession {
        schema: CanonicalSchema::default(),
        identity: SessionIdentity {
            canonical_id: archive.canonical_id.clone(),
            source_title: Some(format!("Compression archive {}", archive.canonical_id)),
        },
        provenance: SessionProvenance {
            imported_at: chrono::Utc::now(),
            imported_by: Some("memorph-cli".to_string()),
            primary_source: ProviderSessionRef {
                provider_id: "memorph".to_string(),
                session_id: archive.summary_event_id.clone(),
                source_path: Some(archive_ref.to_string()),
            },
            aliases: vec![ProviderSessionRef {
                provider_id: archive.source_provider_id.clone(),
                session_id: archive.canonical_id.clone(),
                source_path: None,
            }],
        },
        context: SessionContext {
            workspace_dir: None,
            created_at,
            last_active_at,
            tags: vec!["compression-archive".to_string()],
        },
        events: archive.events,
        artifacts: Vec::new(),
        extensions: BTreeMap::from([("compression_archive".to_string(), archive_value)]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{
        EventBlock, EventLinks, EventMetadata, EventRole, EventSource, MappingDisposition,
        SessionEvent, SessionEventKind,
    };
    use chrono::Utc;

    #[test]
    fn workspace_matches_canonicalizes_equivalent_existing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let session_workspace = dir.path().join(".");
        assert!(workspace_matches(
            "codex",
            session_workspace.to_str(),
            dir.path().to_str(),
        ));
    }

    #[test]
    fn prepare_session_for_target_provider_uses_session_source_provider() {
        let session = sample_opencode_compacted_session();
        let (prepared, report) = prepare_session_for_target_provider(&session, "codex").unwrap();

        assert_eq!(report.normalized_segments, 1);
        assert_eq!(report.target_provider_id, "codex");
        assert!(matches!(
            prepared
                .events
                .first()
                .and_then(|event| event.blocks.first()),
            Some(EventBlock::Compressed { .. })
        ));
    }

    fn sample_opencode_compacted_session() -> CanonicalSession {
        let now = Utc::now();
        CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "s1".to_string(),
                source_title: None,
            },
            provenance: SessionProvenance {
                imported_at: now,
                imported_by: Some("test".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: "opencode".to_string(),
                    session_id: "s1".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext::default(),
            events: vec![
                text_event("old-user", EventRole::User, "large old context", false),
                compaction_event("compact-marker"),
                text_event("summary", EventRole::Assistant, "compressed summary", true),
                text_event("tail", EventRole::User, "new request", false),
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        }
    }

    fn text_event(id: &str, role: EventRole, text: &str, summary: bool) -> SessionEvent {
        let mut provider_ext = BTreeMap::new();
        provider_ext.insert(
            "opencode_message".to_string(),
            serde_json::json!({ "summary": summary }),
        );
        SessionEvent {
            id: id.to_string(),
            kind: SessionEventKind::Message,
            role,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Text {
                text: text.to_string(),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "opencode".to_string(),
                    original_id: Some(id.to_string()),
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext,
            },
        }
    }

    fn compaction_event(id: &str) -> SessionEvent {
        SessionEvent {
            id: id.to_string(),
            kind: SessionEventKind::Unknown,
            role: EventRole::User,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::ProviderPayload {
                kind: "compaction".to_string(),
                payload: serde_json::json!({ "type": "compaction" }),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "opencode".to_string(),
                    original_id: Some(id.to_string()),
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: BTreeMap::new(),
            },
        }
    }
}
