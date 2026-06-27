use anyhow::{Context, Result};
use chrono::Utc;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::canonical::{
    CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole, EventSource,
    MappingDisposition, SessionEvent, SessionEventKind,
};
use crate::config;
use crate::logging;
use crate::provider::{self, canonical_event_text};

const ARCHIVE_SCHEME: &str = "memorph-archive://";
const ARCHIVE_VERSION: u32 = 1;
const ARCHIVE_EXTENSION: &str = "json.gz";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionMode {
    Preserve,
    Expand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressedSegment<'a> {
    pub source_provider_id: &'a str,
    pub summary: &'a str,
    pub source_event_ids: &'a [String],
    pub source_event_count: Option<usize>,
    pub archive_ref: Option<&'a str>,
}

type NormalizeSourceFn = fn(
    &CanonicalSession,
    &CompressionPolicy,
    Option<&Path>,
    CompressionReport,
) -> Result<(CanonicalSession, CompressionReport)>;

struct CompressionSourceAdapter {
    normalize: Option<NormalizeSourceFn>,
}

struct CompressionProviderAdapter {
    provider_id: &'static str,
    source: CompressionSourceAdapter,
}

const COMPRESSION_PROVIDER_ADAPTERS: &[CompressionProviderAdapter] =
    &[CompressionProviderAdapter {
        provider_id: "opencode",
        source: CompressionSourceAdapter {
            normalize: Some(normalize_opencode_source_compression),
        },
    }];

fn provider_adapter(provider_id: &str) -> Option<&'static CompressionProviderAdapter> {
    COMPRESSION_PROVIDER_ADAPTERS
        .iter()
        .find(|adapter| adapter.provider_id == provider_id)
}

pub fn compressed_segment(event: &SessionEvent) -> Option<CompressedSegment<'_>> {
    event.blocks.iter().find_map(|block| {
        if let EventBlock::Compressed {
            source_provider_id,
            summary,
            source_event_ids,
            source_event_count,
            archive_ref,
        } = block
        {
            Some(CompressedSegment {
                source_provider_id: source_provider_id.as_str(),
                summary: summary.as_str(),
                source_event_ids: source_event_ids.as_slice(),
                source_event_count: *source_event_count,
                archive_ref: archive_ref.as_deref(),
            })
        } else {
            None
        }
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompressionPolicy {
    pub mode: CompressionMode,
    pub source_provider_id: String,
    pub target_provider_id: String,
}

impl CompressionPolicy {
    pub fn preserve(source_provider_id: &str, target_provider_id: &str) -> Self {
        Self {
            mode: CompressionMode::Preserve,
            source_provider_id: source_provider_id.to_string(),
            target_provider_id: target_provider_id.to_string(),
        }
    }

    pub fn expand(source_provider_id: &str, target_provider_id: &str) -> Self {
        Self {
            mode: CompressionMode::Expand,
            source_provider_id: source_provider_id.to_string(),
            target_provider_id: target_provider_id.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompressionReport {
    pub normalized_segments: usize,
    pub expanded_segments: usize,
    pub preserved_events: usize,
    pub removed_expanded_events: usize,
    pub restored_events: usize,
    pub target_provider_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archive_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionArchive {
    pub version: u32,
    pub created_at: chrono::DateTime<Utc>,
    pub canonical_id: String,
    pub source_provider_id: String,
    pub target_provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    pub summary_event_id: String,
    pub source_event_ids: Vec<String>,
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompressionArchiveSummary {
    pub archive_ref: String,
    pub created_at: chrono::DateTime<Utc>,
    pub canonical_id: String,
    pub source_provider_id: String,
    pub target_provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    pub summary_event_id: String,
    pub source_event_count: usize,
    pub original_size_bytes: u64,
    pub stored_size_bytes: u64,
    pub compression_ratio: f64,
}

#[derive(Deserialize)]
struct ArchiveHeader {
    #[allow(dead_code)]
    version: u32,
    created_at: chrono::DateTime<Utc>,
    canonical_id: String,
    source_provider_id: String,
    target_provider_id: String,
    #[serde(default)]
    workspace_dir: Option<String>,
    summary_event_id: String,
    source_event_ids: Vec<String>,
    #[allow(dead_code)]
    events: IgnoredAny,
}

#[derive(Debug)]
struct ArchiveSummaryHeader {
    #[allow(dead_code)]
    version: u32,
    created_at: chrono::DateTime<Utc>,
    canonical_id: String,
    source_provider_id: String,
    target_provider_id: String,
    workspace_dir: Option<String>,
    summary_event_id: String,
    source_event_count: usize,
}

pub fn prepare_for_export(
    session: &CanonicalSession,
    policy: &CompressionPolicy,
) -> (CanonicalSession, CompressionReport) {
    match policy.mode {
        CompressionMode::Preserve => preserve_compressed_segments(session, policy),
        CompressionMode::Expand => (
            session.clone(),
            CompressionReport {
                preserved_events: session.events.len(),
                target_provider_id: policy.target_provider_id.clone(),
                ..CompressionReport::default()
            },
        ),
    }
}

pub fn prepare_for_export_with_archive(
    session: &CanonicalSession,
    policy: &CompressionPolicy,
) -> Result<(CanonicalSession, CompressionReport)> {
    let archive_dir = archive_base_dir()?;
    prepare_for_export_with_archive_dir(session, policy, &archive_dir)
}

pub fn load_archive(archive_ref: &str) -> Result<CompressionArchive> {
    load_archive_from_dir(&archive_base_dir()?, archive_ref)
}

pub(crate) fn load_archive_from_dir(
    archive_dir: &Path,
    archive_ref: &str,
) -> Result<CompressionArchive> {
    let path = archive_path_from_ref_in_dir(archive_dir, archive_ref)?;
    let raw = read_archive_text(&path)?;
    serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse compression archive: {}", path.display()))
}

pub fn list_archives() -> Result<Vec<CompressionArchiveSummary>> {
    list_archives_in_dir(&archive_base_dir()?, None)
}

pub fn list_archives_for_workspace(
    workspace: Option<&str>,
) -> Result<Vec<CompressionArchiveSummary>> {
    list_archives_in_dir(&archive_base_dir()?, workspace)
}

pub(crate) fn write_active_compression_archive_in_dir(
    archive_dir: &Path,
    session: &CanonicalSession,
    source_provider_id: &str,
    target_provider_id: &str,
    summary_event: &SessionEvent,
    source_event_ids: Vec<String>,
    events: Vec<SessionEvent>,
) -> Result<String> {
    let policy = CompressionPolicy::preserve(source_provider_id, target_provider_id);
    write_archive(
        archive_dir,
        session,
        &policy,
        summary_event,
        source_event_ids,
        events,
    )
}

#[cfg(test)]
pub(crate) fn expand_compressed_segments_in_dir(
    session: &CanonicalSession,
    source_provider_id: &str,
    target_provider_id: &str,
    archive_dir: &Path,
) -> Result<(CanonicalSession, CompressionReport)> {
    let policy = CompressionPolicy::expand(source_provider_id, target_provider_id);
    expand_compressed_segments_with_archive(session, &policy, archive_dir)
}

fn prepare_for_export_with_archive_dir(
    session: &CanonicalSession,
    policy: &CompressionPolicy,
    archive_dir: &Path,
) -> Result<(CanonicalSession, CompressionReport)> {
    match policy.mode {
        CompressionMode::Preserve => {
            preserve_compressed_segments_with_archive(session, policy, Some(archive_dir))
        }
        CompressionMode::Expand => {
            expand_compressed_segments_with_archive(session, policy, archive_dir)
        }
    }
}

fn preserve_compressed_segments(
    session: &CanonicalSession,
    policy: &CompressionPolicy,
) -> (CanonicalSession, CompressionReport) {
    preserve_compressed_segments_with_archive(session, policy, None)
        .unwrap_or_else(|_| unchanged_report(session, policy))
}

fn preserve_compressed_segments_with_archive(
    session: &CanonicalSession,
    policy: &CompressionPolicy,
    archive_dir: Option<&Path>,
) -> Result<(CanonicalSession, CompressionReport)> {
    let (portable_session, portable_report) =
        normalize_portable_compressed_segments(session, policy);

    normalize_native_source_compression(&portable_session, policy, archive_dir, portable_report)
}

fn normalize_native_source_compression(
    session: &CanonicalSession,
    policy: &CompressionPolicy,
    archive_dir: Option<&Path>,
    base_report: CompressionReport,
) -> Result<(CanonicalSession, CompressionReport)> {
    let Some(adapter) = provider_adapter(&policy.source_provider_id) else {
        return Ok((session.clone(), base_report));
    };
    let Some(normalize) = adapter.source.normalize else {
        return Ok((session.clone(), base_report));
    };
    normalize(session, policy, archive_dir, base_report)
}

fn normalize_opencode_source_compression(
    session: &CanonicalSession,
    policy: &CompressionPolicy,
    archive_dir: Option<&Path>,
    base_report: CompressionReport,
) -> Result<(CanonicalSession, CompressionReport)> {
    let Some(compaction_idx) = session.events.iter().rposition(has_opencode_compaction) else {
        return Ok((session.clone(), base_report));
    };

    let Some(summary_idx) = session
        .events
        .iter()
        .enumerate()
        .skip(compaction_idx + 1)
        .find_map(|(idx, event)| is_opencode_summary_event(event).then_some(idx))
    else {
        return Ok((session.clone(), base_report));
    };

    let summary = canonical_event_text(&session.events[summary_idx]);
    if summary.trim().is_empty() {
        return Ok((session.clone(), base_report));
    }

    let existing_projection = opencode_compaction_projection(&session.events[compaction_idx]);
    let (source_provider_id, source_event_ids, source_event_count, archive_ref) =
        if let Some(projection) = existing_projection {
            (
                projection.source_provider_id,
                projection.source_event_ids,
                projection.source_event_count,
                projection.archive_ref,
            )
        } else {
            let archived_events = session.events[..=compaction_idx].to_vec();
            let source_event_ids = session.events[..=compaction_idx]
                .iter()
                .map(|event| event.id.clone())
                .collect::<Vec<_>>();
            let archive_ref = if let Some(archive_dir) = archive_dir {
                Some(write_archive(
                    archive_dir,
                    session,
                    policy,
                    &session.events[summary_idx],
                    source_event_ids.clone(),
                    archived_events,
                )?)
            } else {
                None
            };
            (
                policy.source_provider_id.clone(),
                source_event_ids.clone(),
                Some(source_event_ids.len()),
                archive_ref,
            )
        };
    let compressed_event = compressed_summary_event(
        &session.events[summary_idx],
        &source_provider_id,
        summary,
        source_event_ids,
        source_event_count,
        archive_ref.clone(),
    );

    let mut next = session.clone();
    let removed_expanded_events = summary_idx;
    let mut events = Vec::with_capacity(1 + session.events.len().saturating_sub(summary_idx + 1));
    events.push(compressed_event);
    events.extend(session.events.iter().skip(summary_idx + 1).cloned());
    next.events = events;

    Ok((
        next,
        CompressionReport {
            normalized_segments: base_report.normalized_segments + 1,
            preserved_events: session.events.len().saturating_sub(removed_expanded_events),
            removed_expanded_events,
            target_provider_id: policy.target_provider_id.clone(),
            archive_refs: base_report
                .archive_refs
                .into_iter()
                .chain(archive_ref)
                .collect(),
            ..CompressionReport::default()
        },
    ))
}

fn normalize_portable_compressed_segments(
    session: &CanonicalSession,
    policy: &CompressionPolicy,
) -> (CanonicalSession, CompressionReport) {
    let mut normalized_segments = 0usize;
    let mut archive_refs = Vec::new();
    let events = session
        .events
        .iter()
        .map(|event| {
            if event
                .blocks
                .iter()
                .any(|block| matches!(block, EventBlock::Compressed { .. }))
            {
                return event.clone();
            }
            let text = canonical_event_text(event);
            let Some(portable) = parse_portable_compressed_text(&text) else {
                return event.clone();
            };
            normalized_segments += 1;
            if let Some(archive_ref) = &portable.archive_ref {
                archive_refs.push(archive_ref.clone());
            }
            compressed_event_from_portable_text(event, portable)
        })
        .collect::<Vec<_>>();

    let mut next = session.clone();
    next.events = events;
    (
        next,
        CompressionReport {
            normalized_segments,
            preserved_events: session.events.len(),
            target_provider_id: policy.target_provider_id.clone(),
            archive_refs,
            ..CompressionReport::default()
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PortableCompressedText {
    source_provider_id: String,
    summary: String,
    source_event_count: Option<usize>,
    archive_ref: Option<String>,
}

fn parse_portable_compressed_text(text: &str) -> Option<PortableCompressedText> {
    let mut lines = text.lines();
    let first = lines.next()?.trim();
    let source_provider_id = first
        .strip_prefix("[Compressed session segment from ")?
        .strip_suffix(']')?
        .trim();
    if source_provider_id.is_empty() {
        return None;
    }

    let mut summary_lines = Vec::new();
    let mut source_event_count = None;
    let mut archive_ref = None;
    for line in lines {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Source event count:") {
            source_event_count = value.trim().parse::<usize>().ok();
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("Archive:") {
            archive_ref = Some(value.trim().to_string()).filter(|value| !value.is_empty());
            continue;
        }
        if is_portable_retrieval_hint_line(trimmed) {
            continue;
        }
        summary_lines.push(line);
    }

    let summary = trim_summary_lines(&summary_lines);
    if summary.is_empty() {
        return None;
    }

    Some(PortableCompressedText {
        source_provider_id: source_provider_id.to_string(),
        summary,
        source_event_count,
        archive_ref,
    })
}

fn is_portable_retrieval_hint_line(line: &str) -> bool {
    line.starts_with("Retrieve specific details with: memorph compression retrieve ")
}

fn trim_summary_lines(lines: &[&str]) -> String {
    let first = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let last = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|idx| idx + 1)
        .unwrap_or(first);
    lines[first..last].join("\n")
}

fn compressed_event_from_portable_text(
    event: &SessionEvent,
    portable: PortableCompressedText,
) -> SessionEvent {
    let mut next = event.clone();
    next.blocks = vec![EventBlock::Compressed {
        source_provider_id: portable.source_provider_id.clone(),
        summary: portable.summary,
        source_event_ids: Vec::new(),
        source_event_count: portable.source_event_count,
        archive_ref: portable.archive_ref.clone(),
    }];
    next.metadata.fidelity = MappingDisposition::Normalized;
    next.metadata.source.phase = Some("compression-portable".to_string());
    next.metadata.provider_ext.insert(
        "memorph_compression".to_string(),
        serde_json::json!({
            "source_provider_id": portable.source_provider_id,
            "source_event_count": portable.source_event_count,
            "archive_ref": portable.archive_ref,
            "portable": true,
        }),
    );
    next
}

fn unchanged_report(
    session: &CanonicalSession,
    policy: &CompressionPolicy,
) -> (CanonicalSession, CompressionReport) {
    (
        session.clone(),
        CompressionReport {
            preserved_events: session.events.len(),
            target_provider_id: policy.target_provider_id.clone(),
            ..CompressionReport::default()
        },
    )
}

fn expand_compressed_segments_with_archive(
    session: &CanonicalSession,
    policy: &CompressionPolicy,
    archive_dir: &Path,
) -> Result<(CanonicalSession, CompressionReport)> {
    let mut next = session.clone();
    let mut events = Vec::with_capacity(session.events.len());
    let mut expanded_segments = 0usize;
    let mut restored_events = 0usize;
    let mut archive_refs = Vec::new();

    for event in &session.events {
        let Some((summary, archive_ref)) = compressed_block_summary_and_archive(event) else {
            events.push(event.clone());
            continue;
        };
        let Some(archive_ref) = archive_ref else {
            events.push(event.clone());
            continue;
        };

        let archive = load_archive_from_dir(archive_dir, archive_ref)?;
        restored_events += archive.events.len();
        events.extend(archive.events);
        events.push(expanded_summary_event(event, summary));
        archive_refs.push(archive_ref.to_string());
        expanded_segments += 1;
    }

    next.events = events;
    let preserved_events = next.events.len();
    Ok((
        next,
        CompressionReport {
            expanded_segments,
            preserved_events,
            restored_events,
            target_provider_id: policy.target_provider_id.clone(),
            archive_refs,
            ..CompressionReport::default()
        },
    ))
}

fn compressed_block_summary_and_archive(event: &SessionEvent) -> Option<(&str, Option<&str>)> {
    event.blocks.iter().find_map(|block| {
        if let EventBlock::Compressed {
            summary,
            archive_ref,
            ..
        } = block
        {
            Some((summary.as_str(), archive_ref.as_deref()))
        } else {
            None
        }
    })
}

fn expanded_summary_event(event: &SessionEvent, summary: &str) -> SessionEvent {
    let mut expanded = event.clone();
    expanded.blocks = vec![EventBlock::Text {
        text: summary.to_string(),
    }];
    expanded.metadata.source.phase = Some("compression-expanded".to_string());
    expanded.metadata.fidelity = MappingDisposition::Normalized;
    expanded
}

fn compressed_summary_event(
    summary_event: &SessionEvent,
    source_provider_id: &str,
    summary: String,
    source_event_ids: Vec<String>,
    source_event_count: Option<usize>,
    archive_ref: Option<String>,
) -> SessionEvent {
    let source_event_count = source_event_count
        .or_else(|| (!source_event_ids.is_empty()).then_some(source_event_ids.len()));
    let mut provider_ext = BTreeMap::new();
    provider_ext.insert(
        "memorph_compression".to_string(),
        serde_json::json!({
            "source_provider_id": source_provider_id,
            "source_event_count": source_event_count,
            "created_from_event_id": summary_event.id,
            "archive_ref": archive_ref,
        }),
    );

    SessionEvent {
        id: format!("memorph-compressed-{}", summary_event.id),
        kind: SessionEventKind::Message,
        role: EventRole::Assistant,
        timestamp: summary_event.timestamp,
        links: EventLinks::default(),
        blocks: vec![EventBlock::Compressed {
            source_provider_id: source_provider_id.to_string(),
            summary,
            source_event_count,
            source_event_ids,
            archive_ref,
        }],
        metadata: EventMetadata {
            source: EventSource {
                provider_id: "memorph".to_string(),
                original_id: Some(summary_event.id.clone()),
                original_role: Some("assistant".to_string()),
                phase: Some("compression".to_string()),
            },
            model: summary_event.metadata.model.clone(),
            usage: None,
            fidelity: MappingDisposition::Normalized,
            provider_ext,
        },
    }
}

fn write_archive(
    archive_dir: &Path,
    session: &CanonicalSession,
    policy: &CompressionPolicy,
    summary_event: &SessionEvent,
    source_event_ids: Vec<String>,
    events: Vec<SessionEvent>,
) -> Result<String> {
    let canonical_id = safe_path_segment(&session.identity.canonical_id);
    let archive_id = archive_id(session, policy, summary_event);
    let dir = archive_dir.join(&canonical_id);
    std::fs::create_dir_all(&dir).with_context(|| {
        format!(
            "Failed to create compression archive dir: {}",
            dir.display()
        )
    })?;
    let filename = format!("{}.{}", archive_id, ARCHIVE_EXTENSION);
    let path = dir.join(&filename);
    let record = CompressionArchive {
        version: ARCHIVE_VERSION,
        created_at: Utc::now(),
        canonical_id: session.identity.canonical_id.clone(),
        source_provider_id: policy.source_provider_id.clone(),
        target_provider_id: policy.target_provider_id.clone(),
        workspace_dir: session.context.workspace_dir.clone(),
        summary_event_id: summary_event.id.clone(),
        source_event_ids,
        events,
    };
    let content = serde_json::to_vec(&record)?;
    write_gzip_atomic(&path, &content)?;
    Ok(format!("{}{}/{}", ARCHIVE_SCHEME, canonical_id, filename))
}

fn read_archive_text(path: &Path) -> Result<String> {
    if path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.ends_with(".json.gz"))
    {
        let file = std::fs::File::open(path)
            .with_context(|| format!("Failed to read compression archive: {}", path.display()))?;
        let mut decoder = GzDecoder::new(file);
        let mut raw = String::new();
        decoder
            .read_to_string(&mut raw)
            .with_context(|| format!("Failed to decompress archive: {}", path.display()))?;
        return Ok(raw);
    }

    std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read compression archive: {}", path.display()))
}

/// Decompress only the first `limit` bytes of a gzip archive to extract metadata.
fn read_archive_header_text(path: &Path, limit: usize) -> Result<String> {
    if path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.ends_with(".json.gz"))
    {
        let file = File::open(path)
            .with_context(|| format!("Failed to read compression archive: {}", path.display()))?;
        let mut decoder = GzDecoder::new(BufReader::new(file));
        let mut buf = vec![0u8; limit];
        let n = decoder
            .read(&mut buf)
            .with_context(|| format!("Failed to decompress archive prefix: {}", path.display()))?;
        buf.truncate(n);
        String::from_utf8(buf)
            .with_context(|| format!("Archive prefix is not UTF-8: {}", path.display()))
    } else {
        let mut file = File::open(path)
            .with_context(|| format!("Failed to read compression archive: {}", path.display()))?;
        let mut buf = vec![0u8; limit];
        let n = file
            .read(&mut buf)
            .with_context(|| format!("Failed to read archive prefix: {}", path.display()))?;
        buf.truncate(n);
        String::from_utf8(buf)
            .with_context(|| format!("Archive prefix is not UTF-8: {}", path.display()))
    }
}

/// Parse a partial JSON document as an archive summary header.
///
/// The header fields appear before `events`, so we can stop after reading the top-level object
/// keys we care about. We append `,"events":[]}` to terminate the object if it is incomplete.
fn parse_summary_header(prefix: &str) -> Result<ArchiveSummaryHeader> {
    let trimmed = prefix.trim_end();
    let json = if trimmed.ends_with('}') {
        trimmed.to_string()
    } else {
        // Object was truncated after a field; ensure it is valid JSON for the fields we need.
        let mut fixed = trimmed.to_string();
        // Strip a trailing comma if present to keep JSON valid.
        if fixed.ends_with(',') {
            fixed.pop();
        }
        fixed.push_str("}}");
        fixed
    };
    let header: ArchiveHeader = serde_json::from_str(&json)
        .with_context(|| "Failed to parse archive header from prefix")?;
    Ok(ArchiveSummaryHeader {
        version: header.version,
        created_at: header.created_at,
        canonical_id: header.canonical_id,
        source_provider_id: header.source_provider_id,
        target_provider_id: header.target_provider_id,
        workspace_dir: header.workspace_dir,
        summary_event_id: header.summary_event_id,
        source_event_count: header.source_event_ids.len(),
    })
}

fn write_gzip_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(content)
        .with_context(|| format!("Failed to compress archive: {}", path.display()))?;
    let compressed = encoder
        .finish()
        .with_context(|| format!("Failed to finish archive compression: {}", path.display()))?;
    let parent = path
        .parent()
        .with_context(|| format!("Path has no parent directory: {}", path.display()))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed to create temporary file in {}", parent.display()))?;
    file.write_all(&compressed)
        .with_context(|| format!("Failed to write temporary archive for {}", path.display()))?;
    file.as_file()
        .sync_all()
        .with_context(|| format!("Failed to sync temporary archive for {}", path.display()))?;
    file.persist(path)
        .map_err(|err| err.error)
        .with_context(|| format!("Failed to persist archive to {}", path.display()))?;
    Ok(())
}

fn archive_base_dir() -> Result<PathBuf> {
    Ok(config::memorph_dir()?.join("compression_archives"))
}

fn archive_path_from_ref_in_dir(archive_dir: &Path, archive_ref: &str) -> Result<PathBuf> {
    let relative = archive_ref
        .strip_prefix(ARCHIVE_SCHEME)
        .with_context(|| format!("Unsupported compression archive ref: {}", archive_ref))?;
    let relative_path = Path::new(relative);
    if relative.is_empty()
        || relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        anyhow::bail!("Invalid compression archive ref: {}", archive_ref);
    }
    Ok(archive_dir.join(relative_path))
}

/// Number of bytes to decompress from the start of each gzip archive to extract the header.
const ARCHIVE_HEADER_READ_LIMIT: usize = 2 * 1024;

/// Build a deterministic cache file path inside the archive directory.
fn archive_summary_index_path(archive_dir: &Path) -> PathBuf {
    archive_dir.join(".summary_index.json")
}

/// In-memory cache for the archive summary index, keyed by archive directory path.
#[derive(Clone, Debug)]
struct IndexCacheEntry {
    summaries: Vec<CompressionArchiveSummary>,
    generated_at: Instant,
}

static INDEX_CACHE: OnceLock<Arc<RwLock<HashMap<PathBuf, IndexCacheEntry>>>> = OnceLock::new();

const INDEX_CACHE_TTL: Duration = Duration::from_secs(30);

fn index_cache() -> Arc<RwLock<HashMap<PathBuf, IndexCacheEntry>>> {
    INDEX_CACHE
        .get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
        .clone()
}

fn cached_summaries_for_dir(archive_dir: &Path) -> Option<Vec<CompressionArchiveSummary>> {
    let cache = index_cache();
    let data = cache.read().ok()?;
    let entry = data.get(archive_dir)?;
    if entry.generated_at.elapsed() < INDEX_CACHE_TTL {
        Some(entry.summaries.clone())
    } else {
        None
    }
}

fn set_cached_summaries_for_dir(archive_dir: &Path, summaries: Vec<CompressionArchiveSummary>) {
    if let Ok(mut cache) = index_cache().write() {
        cache.insert(
            archive_dir.to_path_buf(),
            IndexCacheEntry {
                summaries,
                generated_at: Instant::now(),
            },
        );
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ArchiveSummaryIndexEntry {
    archive_ref: String,
    created_at: chrono::DateTime<Utc>,
    canonical_id: String,
    source_provider_id: String,
    target_provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace_dir: Option<String>,
    summary_event_id: String,
    source_event_count: usize,
    original_size_bytes: u64,
    stored_size_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ArchiveSummaryIndex {
    generated_at: chrono::DateTime<Utc>,
    entries: Vec<ArchiveSummaryIndexEntry>,
}

impl From<&CompressionArchiveSummary> for ArchiveSummaryIndexEntry {
    fn from(summary: &CompressionArchiveSummary) -> Self {
        Self {
            archive_ref: summary.archive_ref.clone(),
            created_at: summary.created_at,
            canonical_id: summary.canonical_id.clone(),
            source_provider_id: summary.source_provider_id.clone(),
            target_provider_id: summary.target_provider_id.clone(),
            workspace_dir: summary.workspace_dir.clone(),
            summary_event_id: summary.summary_event_id.clone(),
            source_event_count: summary.source_event_count,
            original_size_bytes: summary.original_size_bytes,
            stored_size_bytes: summary.stored_size_bytes,
        }
    }
}

impl From<ArchiveSummaryIndexEntry> for CompressionArchiveSummary {
    fn from(entry: ArchiveSummaryIndexEntry) -> Self {
        Self {
            archive_ref: entry.archive_ref,
            created_at: entry.created_at,
            canonical_id: entry.canonical_id,
            source_provider_id: entry.source_provider_id,
            target_provider_id: entry.target_provider_id,
            workspace_dir: entry.workspace_dir,
            summary_event_id: entry.summary_event_id,
            source_event_count: entry.source_event_count,
            original_size_bytes: entry.original_size_bytes,
            stored_size_bytes: entry.stored_size_bytes,
            compression_ratio: compression_ratio(
                entry.original_size_bytes,
                entry.stored_size_bytes,
            ),
        }
    }
}

fn read_summary_index(archive_dir: &Path) -> Option<ArchiveSummaryIndex> {
    let path = archive_summary_index_path(archive_dir);
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.modified().ok()? < std::fs::metadata(archive_dir).ok()?.modified().ok()? {
        return None;
    }
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_summary_index(archive_dir: &Path, summaries: &[CompressionArchiveSummary]) -> Result<()> {
    let path = archive_summary_index_path(archive_dir);
    let index = ArchiveSummaryIndex {
        generated_at: Utc::now(),
        entries: summaries
            .iter()
            .map(ArchiveSummaryIndexEntry::from)
            .collect(),
    };
    let raw = serde_json::to_vec_pretty(&index)?;
    let mut file = tempfile::NamedTempFile::new_in(archive_dir).with_context(|| {
        format!(
            "Failed to create temporary index file in {}",
            archive_dir.display()
        )
    })?;
    file.write_all(&raw).with_context(|| {
        format!(
            "Failed to write temporary index file for {}",
            path.display()
        )
    })?;
    file.as_file()
        .sync_all()
        .with_context(|| format!("Failed to sync temporary index file for {}", path.display()))?;
    file.persist(&path)
        .map_err(|err| err.error)
        .with_context(|| format!("Failed to persist index file to {}", path.display()))?;
    Ok(())
}

fn read_archive_summary_from_path(
    path: &Path,
    group_name: &str,
) -> Option<CompressionArchiveSummary> {
    let prefix = read_archive_header_text(path, ARCHIVE_HEADER_READ_LIMIT).ok()?;
    let header = parse_summary_header(&prefix).ok()?;
    let stored_size_bytes = path.metadata().ok()?.len();
    let file_name = path.file_name()?.to_str()?;
    Some(CompressionArchiveSummary {
        archive_ref: format!("{}{}/{}", ARCHIVE_SCHEME, group_name, file_name),
        created_at: header.created_at,
        canonical_id: header.canonical_id,
        source_provider_id: header.source_provider_id,
        target_provider_id: header.target_provider_id,
        workspace_dir: header.workspace_dir,
        summary_event_id: header.summary_event_id,
        source_event_count: header.source_event_count,
        original_size_bytes: prefix.len() as u64,
        stored_size_bytes,
        compression_ratio: compression_ratio(prefix.len() as u64, stored_size_bytes),
    })
}

fn collect_archive_paths(archive_dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    let mut result = Vec::new();
    for group_entry in std::fs::read_dir(archive_dir)
        .with_context(|| format!("Failed to read archive dir: {}", archive_dir.display()))?
    {
        let group_entry = group_entry?;
        let group_path = group_entry.path();
        if !group_path.is_dir() {
            continue;
        }
        let Some(group_name) = group_path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        for archive_entry in std::fs::read_dir(&group_path)
            .with_context(|| format!("Failed to read archive group: {}", group_path.display()))?
        {
            let archive_entry = archive_entry?;
            let path = archive_entry.path();
            if !is_supported_archive_path(&path) {
                continue;
            }
            result.push((path, group_name.to_string()));
        }
    }
    Ok(result)
}

fn list_archives_in_dir(
    archive_dir: &Path,
    workspace_filter: Option<&str>,
) -> Result<Vec<CompressionArchiveSummary>> {
    if !archive_dir.exists() {
        return Ok(Vec::new());
    }

    let requested_workspace = workspace_filter
        .map(str::trim)
        .filter(|value| !value.is_empty());

    // 1. Try the in-memory cache first. The cache always holds the unfiltered
    //    set so that different workspace filters can share it.
    if let Some(cached) = cached_summaries_for_dir(archive_dir) {
        return Ok(filter_summaries_by_workspace(cached, requested_workspace));
    }

    // 2. Try the persistent index file (valid if newer than the archive directory).
    if let Some(index) = read_summary_index(archive_dir) {
        let summaries: Vec<CompressionArchiveSummary> =
            index.entries.into_iter().map(Into::into).collect();
        set_cached_summaries_for_dir(archive_dir, summaries.clone());
        return Ok(filter_summaries_by_workspace(
            summaries,
            requested_workspace,
        ));
    }

    // 3. Fall back to scanning files, using header-only reads and parallelism.
    let paths = collect_archive_paths(archive_dir)?;

    let mut summaries: Vec<CompressionArchiveSummary> = thread::scope(|scope| {
        let mut handles = Vec::with_capacity(paths.len());
        for (path, group_name) in paths {
            handles.push(scope.spawn(move || read_archive_summary_from_path(&path, &group_name)));
        }
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok().flatten())
            .collect()
    });

    summaries.sort_by_key(|summary| std::cmp::Reverse(summary.created_at));

    // 4. Persist the index for subsequent requests. The index is unfiltered so
    //    it remains valid regardless of which workspace filter is requested next.
    if let Err(err) = write_summary_index(archive_dir, &summaries) {
        logging::error(
            "compression_summary_index",
            &format!("Failed to write summary index: {err}"),
        );
    }
    set_cached_summaries_for_dir(archive_dir, summaries.clone());

    Ok(filter_summaries_by_workspace(
        summaries,
        requested_workspace,
    ))
}

fn filter_summaries_by_workspace(
    summaries: Vec<CompressionArchiveSummary>,
    requested_workspace: Option<&str>,
) -> Vec<CompressionArchiveSummary> {
    let Some(requested) = requested_workspace else {
        return summaries;
    };
    summaries
        .into_iter()
        .filter(|summary| {
            provider::default_workspace_matches(summary.workspace_dir.as_deref(), Some(requested))
        })
        .collect()
}

fn is_supported_archive_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.ends_with(".json") || value.ends_with(".json.gz"))
}

fn compression_ratio(original_size_bytes: u64, stored_size_bytes: u64) -> f64 {
    if original_size_bytes == 0 {
        1.0
    } else {
        stored_size_bytes as f64 / original_size_bytes as f64
    }
}

fn archive_id(
    session: &CanonicalSession,
    policy: &CompressionPolicy,
    summary_event: &SessionEvent,
) -> String {
    let raw = format!(
        "{}:{}:{}:{}",
        session.identity.canonical_id,
        policy.source_provider_id,
        policy.target_provider_id,
        summary_event.id
    );
    format!("{:x}", md5::compute(raw.as_bytes()))
}

fn safe_path_segment(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

fn has_opencode_compaction(event: &SessionEvent) -> bool {
    event.blocks.iter().any(|block| {
        matches!(
            block,
            EventBlock::ProviderPayload { kind, .. } if kind == "compaction"
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpencodeCompactionProjection {
    source_provider_id: String,
    source_event_ids: Vec<String>,
    source_event_count: Option<usize>,
    archive_ref: Option<String>,
}

fn opencode_compaction_projection(event: &SessionEvent) -> Option<OpencodeCompactionProjection> {
    let projection = event.blocks.iter().find_map(|block| {
        let EventBlock::ProviderPayload { kind, payload } = block else {
            return None;
        };
        if kind != "compaction" {
            return None;
        }
        let memorph = payload.get("memorph")?.as_object()?;
        let source_provider_id = memorph
            .get("sourceProviderID")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)?;
        let source_event_ids = memorph
            .get("sourceEventIDs")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let source_event_count = memorph
            .get("sourceEventCount")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .or_else(|| (!source_event_ids.is_empty()).then_some(source_event_ids.len()));
        let archive_ref = memorph
            .get("archiveRef")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        Some(OpencodeCompactionProjection {
            source_provider_id,
            source_event_ids,
            source_event_count,
            archive_ref,
        })
    })?;

    Some(projection)
}

fn is_opencode_summary_event(event: &SessionEvent) -> bool {
    event
        .metadata
        .provider_ext
        .get("opencode_message")
        .and_then(Value::as_object)
        .and_then(|message| message.get("summary"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{
        CanonicalSchema, ProviderSessionRef, SessionContext, SessionIdentity, SessionProvenance,
    };
    use chrono::{TimeZone, Utc};

    #[test]
    fn preserve_opencode_compaction_replaces_expanded_history_with_compressed_segment() {
        let now = Utc::now();
        let session = CanonicalSession {
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
        };

        let policy = CompressionPolicy::preserve("opencode", "codex");
        let (prepared, report) = prepare_for_export(&session, &policy);

        assert_eq!(report.normalized_segments, 1);
        assert_eq!(report.removed_expanded_events, 2);
        assert_eq!(prepared.events.len(), 2);
        assert!(matches!(
            prepared.events[0].blocks.first(),
            Some(EventBlock::Compressed { summary, .. }) if summary == "compressed summary"
        ));
        assert_eq!(prepared.events[1].id, "tail");
    }

    #[test]
    fn archive_prepare_writes_removed_source_events_and_sets_archive_ref() {
        let session = sample_opencode_compacted_session();
        let policy = CompressionPolicy::preserve("opencode", "codex");
        let temp = tempfile::tempdir().unwrap();

        let (prepared, report) =
            prepare_for_export_with_archive_dir(&session, &policy, temp.path()).unwrap();

        assert_eq!(report.archive_refs.len(), 1);
        let archive_ref = &report.archive_refs[0];
        assert!(archive_ref.starts_with(ARCHIVE_SCHEME));
        assert!(matches!(
            prepared.events[0].blocks.first(),
            Some(EventBlock::Compressed { archive_ref: Some(block_ref), .. }) if block_ref == archive_ref
        ));

        assert!(archive_ref.ends_with(".json.gz"));
        let archive = load_archive_from_dir(temp.path(), archive_ref).unwrap();

        assert_eq!(archive.version, ARCHIVE_VERSION);
        assert_eq!(archive.source_provider_id, "opencode");
        assert_eq!(archive.target_provider_id, "codex");
        assert_eq!(archive.events.len(), 2);
        assert_eq!(archive.source_event_ids, vec!["old-user", "compact-marker"]);
    }

    #[test]
    fn archive_loader_supports_legacy_json_archives() {
        let temp = tempfile::tempdir().unwrap();
        write_test_archive(
            temp.path(),
            "legacy",
            "legacy.json",
            Utc.timestamp_millis_opt(1_700_000_000_000)
                .single()
                .unwrap(),
            "summary-legacy",
            vec!["event-legacy".to_string()],
        );

        let archive =
            load_archive_from_dir(temp.path(), "memorph-archive://legacy/legacy.json").unwrap();

        assert_eq!(archive.canonical_id, "legacy");
        assert_eq!(archive.summary_event_id, "summary-legacy");
        assert_eq!(archive.source_event_ids, vec!["event-legacy"]);
    }

    #[test]
    fn list_archives_reports_gzip_storage_sizes() {
        let session = sample_opencode_compacted_session();
        let policy = CompressionPolicy::preserve("opencode", "codex");
        let temp = tempfile::tempdir().unwrap();

        let (_, report) =
            prepare_for_export_with_archive_dir(&session, &policy, temp.path()).unwrap();
        assert_eq!(report.archive_refs.len(), 1);

        let archives = list_archives_in_dir(temp.path(), None).unwrap();

        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].archive_ref, report.archive_refs[0]);
        assert!(archives[0].archive_ref.ends_with(".json.gz"));
        assert!(archives[0].original_size_bytes > 0);
        assert!(archives[0].stored_size_bytes > 0);
        assert!(archives[0].compression_ratio > 0.0);
    }

    #[test]
    fn explicit_expand_restores_archive_events_without_default_expansion() {
        let session = sample_opencode_compacted_session();
        let temp = tempfile::tempdir().unwrap();
        let preserve_policy = CompressionPolicy::preserve("opencode", "codex");
        let expand_policy = CompressionPolicy::expand("opencode", "codex");

        let (preserved, preserve_report) =
            prepare_for_export_with_archive_dir(&session, &preserve_policy, temp.path()).unwrap();
        assert_eq!(preserve_report.normalized_segments, 1);
        assert_eq!(preserved.events.len(), 2);
        assert!(matches!(
            preserved.events[0].blocks.first(),
            Some(EventBlock::Compressed { .. })
        ));

        let (expanded, expand_report) =
            prepare_for_export_with_archive_dir(&preserved, &expand_policy, temp.path()).unwrap();

        assert_eq!(expand_report.expanded_segments, 1);
        assert_eq!(expand_report.restored_events, 2);
        assert_eq!(expanded.events.len(), 4);
        assert_eq!(expanded.events[0].id, "old-user");
        assert_eq!(expanded.events[1].id, "compact-marker");
        assert_eq!(expanded.events[2].id, "memorph-compressed-summary");
        assert_eq!(expanded.events[3].id, "tail");
        assert!(matches!(
            expanded.events[2].blocks.first(),
            Some(EventBlock::Text { text }) if text == "compressed summary"
        ));
        assert!(!expanded.events.iter().any(|event| {
            event
                .blocks
                .iter()
                .any(|block| matches!(block, EventBlock::Compressed { .. }))
        }));
    }

    #[test]
    fn preserve_recovers_portable_compressed_text_from_non_native_provider() {
        let session = CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "portable-s1".to_string(),
                source_title: None,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("test".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: "codex".to_string(),
                    session_id: "portable-s1".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext::default(),
            events: vec![
                provider_text_event(
                    "codex",
                    "portable",
                    EventRole::Assistant,
                    "[Compressed session segment from opencode]\nportable summary\nSource event count: 42\nArchive: memorph-archive://portable/a.json\nRetrieve specific details with: memorph compression retrieve memorph-archive://portable/a.json --query <terms> --max-results 5",
                    false,
                ),
                provider_text_event("codex", "tail", EventRole::User, "new request", false),
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        };

        let policy = CompressionPolicy::preserve("codex", "opencode");
        let (prepared, report) = prepare_for_export(&session, &policy);

        assert_eq!(report.normalized_segments, 1);
        assert_eq!(report.preserved_events, 2);
        assert_eq!(
            report.archive_refs,
            vec!["memorph-archive://portable/a.json".to_string()]
        );
        assert!(matches!(
            prepared.events[0].blocks.first(),
            Some(EventBlock::Compressed {
                source_provider_id,
                summary,
                source_event_count: Some(42),
                archive_ref: Some(archive_ref),
                ..
            }) if source_provider_id == "opencode"
                && summary == "portable summary"
                && archive_ref == "memorph-archive://portable/a.json"
        ));
        assert_eq!(
            prepared.events[0].metadata.source.phase.as_deref(),
            Some("compression-portable")
        );
        assert_eq!(prepared.events[1].id, "tail");
    }

    #[test]
    fn preserve_opencode_compaction_reuses_memorph_projection_metadata() {
        let now = Utc::now();
        let session = CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "native-roundtrip".to_string(),
                source_title: None,
            },
            provenance: SessionProvenance {
                imported_at: now,
                imported_by: Some("test".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: "opencode".to_string(),
                    session_id: "native-roundtrip".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext::default(),
            events: vec![
                compaction_event_with_memorph_projection(
                    "compact-marker",
                    "kimi",
                    Vec::new(),
                    Some(42),
                    Some("memorph-archive://portable/a.json"),
                ),
                text_event("summary", EventRole::Assistant, "portable summary", true),
                text_event("tail", EventRole::User, "new request", false),
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        };

        let policy = CompressionPolicy::preserve("opencode", "codex");
        let temp = tempfile::tempdir().unwrap();
        let (prepared, report) =
            prepare_for_export_with_archive_dir(&session, &policy, temp.path()).unwrap();

        assert_eq!(report.normalized_segments, 1);
        assert_eq!(report.removed_expanded_events, 1);
        assert_eq!(
            report.archive_refs,
            vec!["memorph-archive://portable/a.json".to_string()]
        );
        assert!(matches!(
            prepared.events[0].blocks.first(),
            Some(EventBlock::Compressed {
                source_provider_id,
                summary,
                source_event_ids,
                source_event_count: Some(42),
                archive_ref: Some(archive_ref),
            }) if source_provider_id == "kimi"
                && summary == "portable summary"
                && source_event_ids.is_empty()
                && archive_ref == "memorph-archive://portable/a.json"
        ));
        assert_eq!(prepared.events[1].id, "tail");
        assert!(list_archives_in_dir(temp.path(), None).unwrap().is_empty());
    }

    #[test]
    fn compressed_segment_exposes_canonical_compression_contract() {
        let event = SessionEvent {
            id: "compressed-source".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::Assistant,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Compressed {
                source_provider_id: "deepseek".to_string(),
                summary: "compressed summary".to_string(),
                source_event_ids: vec!["old-1".to_string()],
                source_event_count: Some(9),
                archive_ref: Some("memorph-archive://x/archive.json.gz".to_string()),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "memorph".to_string(),
                    original_id: None,
                    original_role: None,
                    phase: Some("compression".to_string()),
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Normalized,
                provider_ext: BTreeMap::new(),
            },
        };

        let segment = compressed_segment(&event).expect("canonical compressed segment");
        assert_eq!(segment.source_provider_id, "deepseek");
        assert_eq!(segment.summary, "compressed summary");
        assert_eq!(segment.source_event_ids, ["old-1".to_string()]);
        assert_eq!(segment.source_event_count, Some(9));
        assert_eq!(
            segment.archive_ref,
            Some("memorph-archive://x/archive.json.gz")
        );
    }

    #[test]
    fn list_archives_returns_refs_sorted_by_created_time() {
        let temp = tempfile::tempdir().unwrap();
        write_test_archive(
            temp.path(),
            "canonical-a",
            "a.json",
            Utc.timestamp_millis_opt(1_700_000_000_000)
                .single()
                .unwrap(),
            "summary-a",
            vec!["event-a".to_string()],
        );
        write_test_archive(
            temp.path(),
            "canonical-b",
            "b.json",
            Utc.timestamp_millis_opt(1_800_000_000_000)
                .single()
                .unwrap(),
            "summary-b",
            vec!["event-b1".to_string(), "event-b2".to_string()],
        );

        let archives = list_archives_in_dir(temp.path(), None).unwrap();

        assert_eq!(archives.len(), 2);
        assert_eq!(
            archives[0].archive_ref,
            "memorph-archive://canonical-b/b.json"
        );
        assert_eq!(archives[0].source_event_count, 2);
        assert_eq!(archives[0].summary_event_id, "summary-b");
        assert!(archives[0].original_size_bytes > 0);
        assert!(archives[0].stored_size_bytes > 0);
        assert_eq!(archives[0].compression_ratio, 1.0);
        assert_eq!(
            archives[1].archive_ref,
            "memorph-archive://canonical-a/a.json"
        );
        assert_eq!(archives[1].source_event_count, 1);
    }

    #[test]
    fn archive_ref_rejects_paths_outside_archive_dir() {
        let root = Path::new("/tmp/memorph-archives");
        assert!(archive_path_from_ref_in_dir(root, "memorph-archive:///tmp/escape.json").is_err());
        assert!(
            archive_path_from_ref_in_dir(root, "memorph-archive://canonical/../escape.json")
                .is_err()
        );
        assert!(archive_path_from_ref_in_dir(root, "memorph-archive://./escape.json").is_err());
    }

    #[test]
    fn source_adapter_registry_only_handles_source_normalization() {
        let opencode = provider_adapter("opencode").expect("opencode source adapter");
        assert_eq!(opencode.provider_id, "opencode");
        assert!(opencode.source.normalize.is_some());

        assert!(provider_adapter("codex").is_none());
        assert!(provider_adapter("closed-provider").is_none());
    }

    fn write_test_archive(
        root: &Path,
        canonical_id: &str,
        filename: &str,
        created_at: chrono::DateTime<Utc>,
        summary_event_id: &str,
        source_event_ids: Vec<String>,
    ) {
        let dir = root.join(canonical_id);
        std::fs::create_dir_all(&dir).unwrap();
        let archive = CompressionArchive {
            version: ARCHIVE_VERSION,
            created_at,
            canonical_id: canonical_id.to_string(),
            source_provider_id: "opencode".to_string(),
            target_provider_id: "codex".to_string(),
            workspace_dir: None,
            summary_event_id: summary_event_id.to_string(),
            source_event_ids,
            events: Vec::new(),
        };
        std::fs::write(
            dir.join(filename),
            serde_json::to_string_pretty(&archive).unwrap(),
        )
        .unwrap();
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
        provider_text_event("opencode", id, role, text, summary)
    }

    fn provider_text_event(
        provider_id: &str,
        id: &str,
        role: EventRole,
        text: &str,
        summary: bool,
    ) -> SessionEvent {
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
                    provider_id: provider_id.to_string(),
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

    fn compaction_event_with_memorph_projection(
        id: &str,
        source_provider_id: &str,
        source_event_ids: Vec<String>,
        source_event_count: Option<usize>,
        archive_ref: Option<&str>,
    ) -> SessionEvent {
        SessionEvent {
            id: id.to_string(),
            kind: SessionEventKind::Unknown,
            role: EventRole::User,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::ProviderPayload {
                kind: "compaction".to_string(),
                payload: serde_json::json!({
                    "type": "compaction",
                    "memorph": {
                        "sourceProviderID": source_provider_id,
                        "sourceEventIDs": source_event_ids,
                        "sourceEventCount": source_event_count,
                        "archiveRef": archive_ref,
                    }
                }),
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
