use super::*;

pub(super) fn kiro_sessions_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_KIRO_SESSIONS_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
    {
        return Ok(path);
    }

    let home = dirs::home_dir().context("Unable to locate user home directory")?;
    Ok(home.join(".kiro").join("sessions"))
}

pub(super) fn scan_sessions_in(sessions_root: &Path) -> Result<Vec<ProviderSessionSummary>> {
    let mut seen_session_ids = BTreeMap::new();
    let mut sessions = Vec::new();

    for bucket_dir in sorted_child_directories(sessions_root)? {
        for session_dir in sorted_child_directories(&bucket_dir)? {
            if !has_current_source_files(&session_dir)? {
                continue;
            }
            let summary = session_summary_from_dir(&session_dir)?;
            if let Some(previous) =
                seen_session_ids.insert(summary.session_id.clone(), session_dir.to_path_buf())
            {
                anyhow::bail!(
                    "Ambiguous Kiro session id {}: {} and {}",
                    summary.session_id,
                    previous.display(),
                    session_dir.display()
                );
            }
            sessions.push(summary);
        }
    }

    sessions.sort_by(|left, right| {
        right
            .last_active_at
            .cmp(&left.last_active_at)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    Ok(sessions)
}

pub(super) fn sorted_child_directories(parent: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("Failed to read Kiro source directory: {}", parent.display())
            })
        }
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry
            .with_context(|| format!("Failed to read Kiro source entry: {}", parent.display()))?;
        if entry.file_type()?.is_dir() {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

pub(super) fn has_current_source_files(session_dir: &Path) -> Result<bool> {
    Ok(required_regular_file(&session_dir.join("session.json"))?
        && required_regular_file(&session_dir.join("messages.jsonl"))?)
}

pub(super) fn required_regular_file(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            Ok(true)
        }
        Ok(_) => anyhow::bail!("Kiro source is not a regular file: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("Failed to inspect Kiro source: {}", path.display()))
        }
    }
}

pub(super) fn find_session_dirs(session_id: &str) -> Result<Vec<PathBuf>> {
    validate_session_id(session_id)?;
    let sessions_root = kiro_sessions_dir()?;
    if !sessions_root.exists() {
        return Ok(Vec::new());
    }
    let mut matches = Vec::new();
    for bucket_dir in sorted_child_directories(&sessions_root)? {
        let session_dir = bucket_dir.join(session_id);
        let metadata = match std::fs::symlink_metadata(&session_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to inspect Kiro session: {}", session_dir.display())
                })
            }
        };
        if metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && has_current_source_files(&session_dir)?
        {
            matches.push(session_dir);
        }
    }
    matches.sort();
    Ok(matches)
}

pub(super) fn validate_session_id(session_id: &str) -> Result<()> {
    let mut components = Path::new(session_id).components();
    let is_single_normal_component = matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    );
    if !is_single_normal_component {
        anyhow::bail!("Invalid Kiro session id: {session_id}");
    }
    Ok(())
}

pub(super) fn find_session_dir(session_id: &str) -> Result<Option<PathBuf>> {
    let matches = find_session_dirs(session_id)?;
    match matches.as_slice() {
        [] => Ok(None),
        [session_dir] => Ok(Some(session_dir.clone())),
        _ => anyhow::bail!("Kiro session id is ambiguous: {session_id}"),
    }
}

pub(super) fn session_summary_from_dir(session_dir: &Path) -> Result<ProviderSessionSummary> {
    let metadata = read_validated_session_metadata(session_dir)?;
    let title = metadata
        .title
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());
    let project_dir = metadata.workspace_paths.first().cloned();
    let created_at = metadata.created_at.as_deref().and_then(parse_timestamp_ms);
    let last_active_at = metadata
        .last_modified_at
        .as_deref()
        .and_then(parse_timestamp_ms)
        .or_else(|| metadata.created_at.as_deref().and_then(parse_timestamp_ms))
        .or_else(|| source_file_modified_ms(&session_dir.join("messages.jsonl")));

    Ok(ProviderSessionSummary {
        session_id: metadata.id,
        title,
        project_dir,
        created_at,
        last_active_at,
        source_path: Some(session_dir.to_string_lossy().to_string()),
    })
}

pub(super) fn read_validated_session_metadata(session_dir: &Path) -> Result<KiroSessionMetadata> {
    let session_id = session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|id| !id.is_empty())
        .context("Kiro session directory has no valid session id")?;
    let bucket = session_dir
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .context("Kiro session directory has no workspace bucket")?;
    let metadata_path = session_dir.join("session.json");
    let raw = std::fs::read_to_string(&metadata_path)
        .with_context(|| format!("Failed to read Kiro metadata: {}", metadata_path.display()))?;
    let metadata: KiroSessionMetadata = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse Kiro metadata: {}", metadata_path.display()))?;

    if metadata.schema_version != CURRENT_SCHEMA_VERSION
        || metadata.data_model_version != CURRENT_DATA_MODEL_VERSION
    {
        anyhow::bail!(
            "Unsupported Kiro session schema {}/{}: {}",
            metadata.schema_version,
            metadata.data_model_version,
            metadata_path.display()
        );
    }
    if metadata.id != session_id {
        anyhow::bail!(
            "Kiro metadata id {} does not match session directory {}",
            metadata.id,
            session_id
        );
    }
    let expected_bucket = workspace_bucket(&metadata.workspace_paths)?;
    if expected_bucket != bucket {
        anyhow::bail!(
            "Kiro workspace bucket {} does not match metadata workspacePaths (expected {})",
            bucket,
            expected_bucket
        );
    }
    Ok(metadata)
}

pub(super) fn workspace_bucket(workspace_paths: &[String]) -> Result<String> {
    if workspace_paths.is_empty() {
        return Ok("_global".to_string());
    }
    let mut normalized = workspace_paths
        .iter()
        .map(|path| normalize_workspace_path(path))
        .collect::<Result<Vec<_>>>()?;
    normalized.sort();
    let joined = normalized.join("\0");
    let digest = format!("{:x}", Sha256::digest(joined.as_bytes()));
    Ok(digest[..16].to_string())
}

pub(super) fn normalize_workspace_path(path: &str) -> Result<String> {
    if path.is_empty() {
        anyhow::bail!("Kiro workspace path must not be empty");
    }
    if !Path::new(path).is_absolute() {
        anyhow::bail!("Kiro workspace path must be absolute: {path}");
    }
    let normalized = path.replace('\\', "/");
    #[cfg(target_os = "windows")]
    let normalized = normalized.to_lowercase();
    Ok(normalized)
}

pub(super) fn parse_timestamp_ms(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

pub(super) fn source_file_modified_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
}

pub(super) struct KiroImportedEvent {
    event: Event,
    source_file: String,
    source_line: usize,
}

#[derive(Default)]
pub(super) struct KiroStreamImport {
    events: Vec<KiroImportedEvent>,
    sub_execution_parents: BTreeMap<String, String>,
}

pub(super) struct KiroStreamState {
    default_turn_id: Option<String>,
    model_id: Option<String>,
    timestamp_base: DateTime<Utc>,
    active_turn_id: Option<String>,
    tool_call_events: BTreeMap<String, String>,
}

pub(super) fn import_kiro_session_page(
    session_dir: &Path,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<ProviderSessionImportPage> {
    let mut imported = import_canonical_session_from_dir(session_dir)?;
    let full_events = imported.session.events.clone();
    let event_count = full_events.len();
    let message_count = full_events
        .iter()
        .filter(|event| canonical_event_is_visible_message(event))
        .count();
    let full_turns = crate::session_projection::project_session_turns(
        &imported.session.identity.id,
        &full_events,
        TurnQuality::Exact,
    );
    let offset = event_offset.min(event_count);
    imported.session.events = match event_limit {
        Some(limit) => full_events
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect(),
        None => full_events.iter().skip(offset).cloned().collect(),
    };

    let mut turns = crate::session_projection::project_session_turns(
        &imported.session.identity.id,
        &imported.session.events,
        TurnQuality::Inferred,
    );
    let full_event_counts =
        full_events
            .iter()
            .fold(BTreeMap::<String, usize>::new(), |mut counts, event| {
                if let Some(turn_id) = event.links.turn_id.as_ref() {
                    *counts.entry(turn_id.clone()).or_default() += 1;
                }
                counts
            });
    let page_event_counts = imported.session.events.iter().fold(
        BTreeMap::<String, usize>::new(),
        |mut counts, event| {
            if let Some(turn_id) = event.links.turn_id.as_ref() {
                *counts.entry(turn_id.clone()).or_default() += 1;
            }
            counts
        },
    );
    for turn in &mut turns {
        if let Some(turn_id) = turn.provider_turn_id.as_ref() {
            if page_event_counts.get(turn_id) != full_event_counts.get(turn_id) {
                turn.confidence = crate::session_projection::TurnConfidence::Inferred;
            }
        }
    }

    Ok(ProviderSessionImportPage {
        imported,
        event_count,
        message_count,
        turn_count: Some(full_turns.len()),
        turns,
    })
}

pub(super) fn import_canonical_session_from_dir(session_dir: &Path) -> Result<ImportedSession> {
    if kiro_session_source_fingerprint(session_dir)?.is_none() {
        anyhow::bail!(
            "Current Kiro session source is missing required files: {}",
            session_dir.display()
        );
    }

    let metadata = read_validated_session_metadata(session_dir)?;
    let metadata_path = session_dir.join("session.json");
    let raw_metadata: Value =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).with_context(|| {
            format!("Failed to read Kiro metadata: {}", metadata_path.display())
        })?)
        .with_context(|| format!("Failed to parse Kiro metadata: {}", metadata_path.display()))?;
    let created_at = metadata.created_at.as_deref().and_then(parse_utc_timestamp);
    let metadata_last_active_at = metadata
        .last_modified_at
        .as_deref()
        .and_then(parse_utc_timestamp);
    let timestamp_base = created_at.unwrap_or_else(|| DateTime::<Utc>::from(std::time::UNIX_EPOCH));
    let model_id = raw_metadata
        .get("modelId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);

    let messages_path = session_dir.join("messages.jsonl");
    let mut imported = read_kiro_event_stream(
        &messages_path,
        "messages",
        None,
        model_id.as_deref(),
        timestamp_base,
        &mut report,
    )?;

    let sub_executions_dir = session_dir.join("sub-executions");
    if sub_executions_dir.exists() {
        for sub_execution_path in sorted_jsonl_files(&sub_executions_dir)? {
            let sub_execution_id = sub_execution_path
                .file_stem()
                .and_then(|value| value.to_str())
                .context("Kiro sub-execution file has no valid execution id")?
                .to_string();
            let source_file = format!(
                "sub-executions/{}",
                sub_execution_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .context("Kiro sub-execution file has no valid name")?
            );
            let parent_event_id = imported
                .sub_execution_parents
                .get(&sub_execution_id)
                .cloned();
            let mut sub_import = read_kiro_event_stream(
                &sub_execution_path,
                &source_file,
                Some(&sub_execution_id),
                model_id.as_deref(),
                timestamp_base,
                &mut report,
            )?;
            if let Some(parent_event_id) = parent_event_id {
                for event in &mut sub_import.events {
                    event.event.links.parent_event_id = Some(parent_event_id.clone());
                }
            } else if !sub_import.events.is_empty() {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: Fidelity::Preserved,
                    code: "sub_execution_parent_unresolved".to_string(),
                    message: format!(
                        "Preserved Kiro sub-execution {sub_execution_id} by native timestamp without inventing a parent relation"
                    ),
                    path: Some(source_file),
                    raw: None,
                });
            }
            imported.events.extend(sub_import.events);
        }
    }

    imported.events.sort_by(|left, right| {
        left.event
            .timestamp
            .cmp(&right.event.timestamp)
            .then_with(|| left.source_file.cmp(&right.source_file))
            .then_with(|| left.source_line.cmp(&right.source_line))
    });
    let events = imported
        .events
        .into_iter()
        .map(|event| event.event)
        .collect::<Vec<_>>();
    let event_last_active_at = events.iter().map(|event| event.timestamp).max();
    let source_title = metadata
        .title
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());
    let workspace_dir = metadata.workspace_paths.first().cloned();
    let mut extensions = BTreeMap::new();
    extensions.insert("kiro_session_metadata".to_string(), raw_metadata);

    let event_meta = events
        .iter()
        .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
        .collect::<Vec<_>>();
    Ok(ImportedSession {
        session: Session {
            schema: Schema::default(),
            identity: Identity {
                id: metadata.id.clone(),
                title: source_title,
            },
            context: Context {
                workspace: workspace_dir,
                created_at,
                last_active_at: metadata_last_active_at.max(event_last_active_at),
                tags: Vec::new(),
            },
            events,
            extensions,
        },
        provenance: Provenance {
            imported_at: Utc::now(),
            imported_by: Some("memorph-cli".to_string()),
            primary_source: ProviderRef {
                provider_id: PROVIDER_ID.to_string(),
                session_id: metadata.id,
                source_path: Some(session_dir.to_string_lossy().to_string()),
            },
            aliases: Vec::new(),
        },
        event_meta,
        report,
    })
}

pub(super) fn sorted_jsonl_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut files = std::fs::read_dir(directory)
        .with_context(|| {
            format!(
                "Failed to read Kiro source directory: {}",
                directory.display()
            )
        })?
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort_by_key(|entry| entry.file_name());
    files
        .into_iter()
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl"))
        .map(|entry| {
            let path = entry.path();
            source_file_marker(&path)?.with_context(|| {
                format!(
                    "Kiro source disappeared while importing: {}",
                    path.display()
                )
            })?;
            Ok(path)
        })
        .collect()
}

pub(super) fn read_kiro_event_stream(
    path: &Path,
    source_file: &str,
    default_turn_id: Option<&str>,
    model_id: Option<&str>,
    timestamp_base: DateTime<Utc>,
    report: &mut MappingReport,
) -> Result<KiroStreamImport> {
    let file = File::open(path)
        .with_context(|| format!("Failed to read Kiro event stream: {}", path.display()))?;
    let mut imported = KiroStreamImport::default();
    let mut state = KiroStreamState {
        default_turn_id: default_turn_id.map(str::to_string),
        model_id: model_id.map(str::to_string),
        timestamp_base,
        active_turn_id: default_turn_id.map(str::to_string),
        tool_call_events: BTreeMap::new(),
    };

    for (line_index, line) in BufReader::new(file).lines().enumerate() {
        let line_number = line_index + 1;
        let line = line.with_context(|| {
            format!(
                "Failed to read Kiro event stream line: {}:{}",
                path.display(),
                line_number
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(&line) {
            Ok(record) => record,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: Fidelity::Dropped,
                    code: "invalid_jsonl_line".to_string(),
                    message: format!("Failed to parse Kiro event stream line: {error}"),
                    path: Some(format!("{source_file}:line:{line_number}")),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };
        let sub_execution_id = record
            .get("payload")
            .and_then(|payload| payload.get("subExecutionId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let event =
            canonical_event_from_kiro_record(record, source_file, line_number, &mut state, report);
        if let Some(sub_execution_id) = sub_execution_id {
            imported
                .sub_execution_parents
                .insert(sub_execution_id, event.id.clone());
        }
        imported.events.push(KiroImportedEvent {
            event,
            source_file: source_file.to_string(),
            source_line: line_number,
        });
    }

    Ok(imported)
}

pub(super) fn canonical_event_from_kiro_record(
    record: Value,
    source_file: &str,
    line_number: usize,
    state: &mut KiroStreamState,
    report: &mut MappingReport,
) -> Event {
    let path = format!("{source_file}:line:{line_number}");
    let payload = record.get("payload").cloned().unwrap_or(Value::Null);
    let payload_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let original_id = record.get("id").and_then(Value::as_str).map(str::to_string);
    let event_id = match (source_file, original_id.as_deref()) {
        ("messages", Some(id)) => id.to_string(),
        (_, Some(id)) => format!("kiro:{source_file}:{id}"),
        _ => format!("kiro:{source_file}:line:{line_number}"),
    };
    if original_id.is_none() {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: Fidelity::Normalized,
            code: "missing_event_id".to_string(),
            message: "Generated a stable Kiro canonical event id from source location".to_string(),
            path: Some(path.clone()),
            raw: Some(record.clone()),
        });
    }
    let timestamp = record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_utc_timestamp)
        .unwrap_or_else(|| {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: Fidelity::Normalized,
                code: "invalid_event_timestamp".to_string(),
                message: "Used the session timestamp plus source line offset for a Kiro event"
                    .to_string(),
                path: Some(path.clone()),
                raw: record.get("timestamp").cloned(),
            });
            state.timestamp_base + chrono::Duration::milliseconds(line_number as i64)
        });

    // `subExecutionId` identifies a child stream from the main stream; it is
    // not the event's own turn id. Child stream events inherit their native
    // turn id from the file name through `default_turn_id`.
    let explicit_turn_id = payload
        .get("executionId")
        .and_then(Value::as_str)
        .map(str::to_string);
    if payload_type == "turn_start" {
        state.active_turn_id = explicit_turn_id
            .clone()
            .or_else(|| state.default_turn_id.clone());
    }
    let provider_turn_id = explicit_turn_id
        .or_else(|| state.active_turn_id.clone())
        .or_else(|| state.default_turn_id.clone());
    let mut links = Links {
        turn_id: provider_turn_id,
        ..Links::default()
    };

    let (kind, role, blocks) = match payload_type {
        "user" => (
            EventKind::Message,
            Role::User,
            kiro_message_blocks(&payload, false, &path, report),
        ),
        "assistant" => {
            let operation_type = payload
                .get("operationType")
                .and_then(Value::as_str)
                .unwrap_or("Say");
            let reasoning = operation_type.eq_ignore_ascii_case("reasoning");
            if !reasoning && !operation_type.eq_ignore_ascii_case("say") {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: Fidelity::Normalized,
                    code: "unknown_assistant_operation".to_string(),
                    message: format!(
                        "Mapped Kiro assistant operation {operation_type} as visible text"
                    ),
                    path: Some(path.clone()),
                    raw: Some(payload.clone()),
                });
            }
            (
                EventKind::Message,
                Role::Assistant,
                kiro_message_blocks(&payload, reasoning, &path, report),
            )
        }
        "system" => (
            EventKind::Message,
            Role::System,
            kiro_message_blocks(&payload, false, &path, report),
        ),
        "agent_note" => (
            EventKind::Message,
            Role::Assistant,
            kiro_message_blocks(&payload, false, &path, report),
        ),
        "tool_call" => {
            let tool_call_id = payload
                .get("toolCallId")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: Fidelity::Normalized,
                        code: "missing_tool_call_id".to_string(),
                        message: "Generated a stable tool call id from the Kiro event id"
                            .to_string(),
                        path: Some(path.clone()),
                        raw: Some(payload.clone()),
                    });
                    event_id.clone()
                });
            let name = payload
                .get("toolName")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: Fidelity::Normalized,
                        code: "missing_tool_name".to_string(),
                        message: "Used `unknown` because the Kiro tool call had no tool name"
                            .to_string(),
                        path: Some(path.clone()),
                        raw: Some(payload.clone()),
                    });
                    "unknown".to_string()
                });
            state
                .tool_call_events
                .insert(tool_call_id.clone(), event_id.clone());
            (
                EventKind::Action,
                Role::Assistant,
                vec![Block::ToolCall {
                    tool_call_id,
                    name,
                    input: payload.get("args").cloned(),
                }],
            )
        }
        "tool_result" => {
            let tool_call_id = payload
                .get("toolCallId")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: Fidelity::Normalized,
                        code: "missing_tool_result_call_id".to_string(),
                        message: "Used `unknown` because the Kiro tool result had no tool call id"
                            .to_string(),
                        path: Some(path.clone()),
                        raw: Some(payload.clone()),
                    });
                    "unknown".to_string()
                });
            if let Some(parent_event_id) = state.tool_call_events.get(&tool_call_id) {
                links.parent_event_id = Some(parent_event_id.clone());
            }
            let content = kiro_tool_result_content(payload.get("content"), &path, report);
            let is_error = payload.get("success").and_then(Value::as_bool) == Some(false)
                || matches!(
                    payload.get("status").and_then(Value::as_str),
                    Some("error" | "failed")
                );
            (
                EventKind::Observation,
                Role::Tool,
                vec![Block::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                }],
            )
        }
        "turn_start" => {
            (
                EventKind::Lifecycle,
                Role::System,
                vec![Block::Other { raw: payload.clone() }],
            )
        }
        "turn_end" => {
            links.turn_outcome = Some(kiro_turn_end_boundary(&payload));
            (
                EventKind::Lifecycle,
                Role::System,
                vec![Block::Other { raw: payload.clone() }],
            )
        }
        "session_start"
        | "usage_summary"
        | "session_metadata"
        | "session_event"
        | "sub_agent_start"
        | "sub_agent_complete"
        | "sub_agent_progress"
        | "steering_inclusion"
        | "tombstone"
        | "ContextualHookInvoked"
        | "pending_interaction"
        | "interaction_resolved" => (
            EventKind::Lifecycle,
            Role::System,
            vec![Block::Other { raw: payload.clone() }],
        ),
        unknown => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: Fidelity::Preserved,
                code: "unknown_payload_preserved".to_string(),
                message: format!("Preserved unknown Kiro payload type {unknown}"),
                path: Some(path.clone()),
                raw: Some(payload.clone()),
            });
            (
                EventKind::Other,
                Role::Other,
                vec![Block::Other { raw: payload.clone() }],
            )
        }
    };

    if payload_type == "session_start" {
        if let Some(message_id) = payload.get("messageId").and_then(Value::as_str) {
            links.related_event_ids.push(message_id.to_string());
        }
    }
    if payload_type == "turn_end" {
        state.active_turn_id = state.default_turn_id.clone();
    }

    Event {
        id: event_id,
        kind,
        role,
        timestamp,
        links,
        blocks,
        metadata: Metadata {
            model: state.model_id.clone(),
            usage: None,
        },
    }
}

pub(super) fn kiro_message_blocks(
    payload: &Value,
    reasoning: bool,
    path: &str,
    report: &mut MappingReport,
) -> Vec<Block> {
    let Some(content) = payload.get("content") else {
        return vec![Block::Other { raw: payload.clone() }];
    };
    let mut blocks = Vec::new();
    match content {
        Value::String(text) => push_kiro_text_block(&mut blocks, text, reasoning),
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(text) => push_kiro_text_block(&mut blocks, text, reasoning),
                    Value::Object(object) => {
                        if let Some(text) = object.get("text").and_then(Value::as_str) {
                            push_kiro_text_block(&mut blocks, text, reasoning);
                        } else if let Some(thinking) =
                            object.get("thinking").and_then(Value::as_str)
                        {
                            blocks.push(Block::Thinking {
                                text: thinking.to_string(),
                                signature: object
                                    .get("signature")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                            });
                        } else {
                            blocks.push(Block::Other {
                                raw: item.clone(),
                            });
                        }
                    }
                    _ => blocks.push(Block::Other { raw: item.clone() }),
                }
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                push_kiro_text_block(&mut blocks, text, reasoning);
            } else {
                blocks.push(Block::Other {
                    raw: content.clone(),
                });
            }
        }
        _ => blocks.push(Block::Other {
            raw: content.clone(),
        }),
    }
    if blocks.is_empty() {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: Fidelity::Normalized,
            code: "empty_message_content".to_string(),
            message: "Preserved an empty Kiro message as provider payload".to_string(),
            path: Some(path.to_string()),
            raw: Some(payload.clone()),
        });
        blocks.push(Block::Other { raw: payload.clone() });
    }
    blocks
}

pub(super) fn push_kiro_text_block(blocks: &mut Vec<Block>, text: &str, reasoning: bool) {
    if text.is_empty() {
        return;
    }
    if reasoning {
        blocks.push(Block::Thinking {
            text: text.to_string(),
            signature: None,
        });
    } else {
        blocks.push(Block::Text {
            text: text.to_string(),
        });
    }
}

pub(super) fn kiro_tool_result_content(
    content: Option<&Value>,
    path: &str,
    report: &mut MappingReport,
) -> String {
    match content {
        Some(Value::String(content)) => content.clone(),
        Some(Value::Null) | None => String::new(),
        Some(content) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: Fidelity::Normalized,
                code: "structured_tool_result_normalized".to_string(),
                message: "Serialized structured Kiro tool result content as JSON".to_string(),
                path: Some(path.to_string()),
                raw: Some(content.clone()),
            });
            serde_json::to_string(content).unwrap_or_default()
        }
    }
}

pub(super) fn kiro_turn_end_boundary(payload: &Value) -> TurnOutcome {
    match payload
        .get("stopReason")
        .or_else(|| payload.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("end_turn")
    {
        "error" | "failed" | "failure" => TurnOutcome::Failed,
        "interrupted" | "cancelled" | "canceled" | "aborted" => TurnOutcome::Interrupted,
        _ => TurnOutcome::Completed,
    }
}

pub(super) fn parse_utc_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

pub(super) struct SourceFileMarker {
    value: String,
    modified_at_ms: i64,
    size_bytes: i64,
}

pub(super) fn source_file_marker(path: &Path) -> Result<Option<SourceFileMarker>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to inspect Kiro source: {}", path.display()))
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!("Kiro source is not a regular file: {}", path.display());
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok());
    let modified_at_ms = modified
        .as_ref()
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    let modified_at_ns = modified.map(|duration| duration.as_nanos()).unwrap_or(0);
    let size_bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
    Ok(Some(SourceFileMarker {
        value: format!("present:{modified_at_ns}:{size_bytes}"),
        modified_at_ms,
        size_bytes,
    }))
}

pub(super) fn kiro_session_source_fingerprint(
    session_dir: &Path,
) -> Result<Option<ProviderSourceFingerprint>> {
    let sessions_root = kiro_sessions_dir()?;
    if session_dir.parent().and_then(Path::parent) != Some(sessions_root.as_path()) {
        anyhow::bail!(
            "Kiro session source locator is outside the configured sessions root: {}",
            session_dir.display()
        );
    }

    let directory_metadata = match std::fs::symlink_metadata(session_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to inspect Kiro session source: {}",
                    session_dir.display()
                )
            })
        }
    };
    if !directory_metadata.file_type().is_dir() || directory_metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kiro session source locator must be a directory: {}",
            session_dir.display()
        );
    }

    let Some(session_marker) = source_file_marker(&session_dir.join("session.json"))? else {
        return Ok(None);
    };
    let Some(messages_marker) = source_file_marker(&session_dir.join("messages.jsonl"))? else {
        return Ok(None);
    };
    read_validated_session_metadata(session_dir)?;

    let sub_executions_dir = session_dir.join("sub-executions");
    let mut sub_execution_markers = Vec::new();
    let mut modified_at_ms = session_marker
        .modified_at_ms
        .max(messages_marker.modified_at_ms);
    let mut size_bytes = session_marker
        .size_bytes
        .saturating_add(messages_marker.size_bytes);

    match std::fs::symlink_metadata(&sub_executions_dir) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                anyhow::bail!(
                    "Kiro sub-executions source is not a directory: {}",
                    sub_executions_dir.display()
                );
            }
            let mut entries = std::fs::read_dir(&sub_executions_dir)
                .with_context(|| {
                    format!(
                        "Failed to read Kiro sub-executions: {}",
                        sub_executions_dir.display()
                    )
                })?
                .collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl") {
                    continue;
                }
                let marker = source_file_marker(&path)?.with_context(|| {
                    format!(
                        "Kiro sub-execution disappeared while scanning: {}",
                        path.display()
                    )
                })?;
                let name = entry.file_name().to_string_lossy().to_string();
                modified_at_ms = modified_at_ms.max(marker.modified_at_ms);
                size_bytes = size_bytes.saturating_add(marker.size_bytes);
                sub_execution_markers.push(format!("{name}:{}", marker.value));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to inspect Kiro sub-executions: {}",
                    sub_executions_dir.display()
                )
            })
        }
    }

    let sub_execution_value = if sub_execution_markers.is_empty() {
        "absent".to_string()
    } else {
        let joined = sub_execution_markers.join("\0");
        format!(
            "{}:{:x}",
            sub_execution_markers.len(),
            Sha256::digest(joined.as_bytes())
        )
    };

    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms,
        size_bytes,
        value: format!(
            "kiro-v2:session:{}:messages:{}:sub-executions:{}",
            session_marker.value, messages_marker.value, sub_execution_value
        ),
    }))
}
