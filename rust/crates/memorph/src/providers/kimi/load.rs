use super::*;

pub(super) fn get_kimi_sessions_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_KIMI_SESSIONS_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi sessions dir lock")
        .clone()
    {
        return path;
    }

    dirs::home_dir()
        .map(|h| h.join(".kimi").join("sessions"))
        .unwrap_or_else(|| PathBuf::from(".kimi").join("sessions"))
}

pub(super) fn get_kimi_json_path() -> PathBuf {
    #[cfg(test)]
    if let Some(sessions_dir) = TEST_KIMI_SESSIONS_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi sessions dir lock")
        .clone()
    {
        return sessions_dir
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("kimi.json");
    }

    dirs::home_dir()
        .map(|h| h.join(".kimi").join("kimi.json"))
        .unwrap_or_else(|| PathBuf::from(".kimi").join("kimi.json"))
}

pub(super) fn md5_hex(data: &[u8]) -> String {
    use std::fmt::Write;
    let hash = md5::compute(data);
    let mut hex = String::with_capacity(32);
    for byte in hash.as_ref() {
        write!(&mut hex, "{:02x}", byte).unwrap();
    }
    hex
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct KimiState {
    #[serde(default)]
    custom_title: Option<String>,
    #[serde(default)]
    archived: bool,
}

#[derive(Debug, Clone)]
pub(super) struct KimiWorkDir {
    pub(super) project_dir: String,
    mapping_fingerprint: String,
    mapping_size_bytes: i64,
}

pub(super) fn read_state_json(path: &Path) -> Result<KimiState> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read state.json: {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse state.json: {}", path.display()))
}

pub(super) fn load_work_dir_map() -> Result<BTreeMap<String, KimiWorkDir>> {
    let path = get_kimi_json_path();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read kimi.json: {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse kimi.json: {}", path.display()))?;

    let mut map = BTreeMap::new();
    if let Some(dirs) = value.get("work_dirs").and_then(Value::as_array) {
        for entry in dirs {
            let Some(path_str) = entry.get("path").and_then(Value::as_str) else {
                continue;
            };
            let kaos = entry.get("kaos").and_then(Value::as_str).unwrap_or("local");
            let path_hash = md5_hex(path_str.as_bytes());
            let key = if kaos == "local" {
                path_hash
            } else {
                format!("{kaos}_{path_hash}")
            };
            let mapping_bytes = serde_json::to_vec(entry)?;
            let work_dir = KimiWorkDir {
                project_dir: path_str.to_string(),
                mapping_fingerprint: md5_hex(&mapping_bytes),
                mapping_size_bytes: i64::try_from(mapping_bytes.len()).unwrap_or(i64::MAX),
            };
            if map.insert(key.clone(), work_dir).is_some() {
                anyhow::bail!("Duplicate Kimi work-dir key in kimi.json: {key}");
            }
        }
    }
    Ok(map)
}

pub(super) fn find_session_dirs(session_id: &str) -> Result<Vec<PathBuf>> {
    let root = get_kimi_sessions_dir();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut matches = Vec::new();
    for entry in WalkDir::new(&root)
        .max_depth(2)
        .min_depth(2)
        .follow_links(false)
    {
        let entry =
            entry.with_context(|| format!("Failed to walk Kimi sessions: {}", root.display()))?;
        if entry.file_type().is_dir()
            && entry.path().file_name().and_then(|name| name.to_str()) == Some(session_id)
        {
            matches.push(entry.path().to_path_buf());
        }
    }
    matches.sort();
    Ok(matches)
}

pub(super) fn find_session_dir(session_id: &str) -> Result<Option<PathBuf>> {
    let matches = find_session_dirs(session_id)?;
    match matches.as_slice() {
        [] => Ok(None),
        [session_dir] => Ok(Some(session_dir.clone())),
        _ => anyhow::bail!("Kimi session id is ambiguous: {session_id}"),
    }
}

pub(super) fn kimi_session_summary(
    session_dir: &Path,
    session_id: String,
    project_dir: Option<String>,
) -> Result<Option<ProviderSessionSummary>> {
    let state_path = session_dir.join("state.json");
    let (state_title, archived) = if state_path.exists() {
        match read_state_json(&state_path) {
            Ok(state) => (state.custom_title, state.archived),
            Err(_) => (None, false),
        }
    } else {
        (None, false)
    };
    if archived {
        return Ok(None);
    }

    let title = state_title
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
        .or_else(|| first_turn_begin_text(&session_dir.join("wire.jsonl")));
    let created_at = File::open(session_dir.join("wire.jsonl"))
        .ok()
        .and_then(|file| {
            BufReader::new(file)
                .lines()
                .map_while(Result::ok)
                .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
                .find_map(|value| parse_wire_timestamp(&value))
                .map(|timestamp| timestamp.timestamp_millis())
        })
        .or_else(|| file_created_ms(session_dir));
    let last_active_at = file_modified_ms(&session_dir.join("context.jsonl"))?;

    Ok(Some(ProviderSessionSummary {
        session_id,
        title,
        project_dir,
        created_at,
        last_active_at,
        source_path: Some(session_dir.to_string_lossy().to_string()),
    }))
}

pub(super) fn first_turn_begin_text(wire_path: &Path) -> Option<String> {
    let file = File::open(wire_path).ok()?;
    for line in BufReader::new(file).lines().map_while(|line| line.ok()) {
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(message) = value.get("message") else {
            continue;
        };
        if message.get("type").and_then(Value::as_str) != Some("TurnBegin") {
            continue;
        }
        let Some(inputs) = message
            .get("payload")
            .and_then(|payload| payload.get("user_input"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for input in inputs {
            if input.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = input
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

pub(super) fn file_modified_ms(path: &Path) -> Result<Option<i64>> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata_modified_ms(&metadata))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("Failed to read Kimi source metadata: {}", path.display())),
    }
}

fn file_created_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .created()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
}

pub(super) fn metadata_modified_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

pub(super) fn kimi_file_fingerprint(
    path: &Path,
    required: bool,
) -> Result<Option<(String, i64, i64)>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return if required {
                Ok(None)
            } else {
                Ok(Some(("absent".to_string(), 0, 0)))
            };
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("Failed to read Kimi source metadata: {}", path.display())
            })
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!("Kimi source is not a regular file: {}", path.display());
    }
    let modified_at_ms = metadata_modified_ms(&metadata);
    let size_bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
    Ok(Some((
        format!("present:{modified_at_ms}:{size_bytes}"),
        modified_at_ms,
        size_bytes,
    )))
}

pub(super) fn kimi_session_source_fingerprint(
    session_dir: &Path,
) -> Result<Option<ProviderSourceFingerprint>> {
    let metadata = match std::fs::symlink_metadata(session_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to read Kimi session source metadata: {}",
                    session_dir.display()
                )
            })
        }
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kimi session source locator must be a directory: {}",
            session_dir.display()
        );
    }
    let sessions_root = get_kimi_sessions_dir();
    if session_dir.parent().and_then(Path::parent) != Some(sessions_root.as_path()) {
        anyhow::bail!(
            "Kimi session source locator is outside the configured sessions root: {}",
            session_dir.display()
        );
    }

    let Some((context_marker, context_modified_at_ms, context_size_bytes)) =
        kimi_file_fingerprint(&session_dir.join("context.jsonl"), true)?
    else {
        return Ok(None);
    };
    let (wire_marker, wire_modified_at_ms, wire_size_bytes) =
        kimi_file_fingerprint(&session_dir.join("wire.jsonl"), false)?
            .expect("optional Kimi fingerprint marker");
    let (state_marker, state_modified_at_ms, state_size_bytes) =
        kimi_file_fingerprint(&session_dir.join("state.json"), false)?
            .expect("optional Kimi fingerprint marker");

    let work_dir_key = session_dir
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .context("Kimi session source has no work-dir key")?;
    let work_dirs = load_work_dir_map()?;
    let (mapping_marker, mapping_size_bytes) = work_dirs
        .get(work_dir_key)
        .map(|work_dir| {
            (
                format!("present:{}", work_dir.mapping_fingerprint),
                work_dir.mapping_size_bytes,
            )
        })
        .unwrap_or_else(|| ("absent".to_string(), 0));
    let kimi_json_modified_at_ms = file_modified_ms(&get_kimi_json_path())?.unwrap_or(0);
    let modified_at_ms = context_modified_at_ms
        .max(wire_modified_at_ms)
        .max(state_modified_at_ms)
        .max(kimi_json_modified_at_ms);
    let size_bytes = context_size_bytes
        .saturating_add(wire_size_bytes)
        .saturating_add(state_size_bytes)
        .saturating_add(mapping_size_bytes);

    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms,
        size_bytes,
        value: format!(
            "kimi-v1:context:{context_marker}:wire:{wire_marker}:state:{state_marker}:mapping:{mapping_marker}"
        ),
    }))
}

pub(super) fn import_kimi_session_page(
    session_dir: &Path,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<ProviderSessionImportPage> {
    let mut imported = import_canonical_session_from_dir(session_dir)?;
    let event_count = imported.session.events.len();
    let message_count = imported
        .session
        .events
        .iter()
        .filter(|event| event_is_visible_message(event))
        .count();
    let turn_count = crate::session_projection::project_session_turns(
        &imported.session.identity.id,
        &imported.session.events,
        TurnQuality::Inferred,
    )
    .len();
    let offset = event_offset.min(event_count);
    imported.session.events = match event_limit {
        Some(limit) => imported
            .session
            .events
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect(),
        None => imported.session.events.into_iter().skip(offset).collect(),
    };
    let turns = crate::session_projection::project_session_turns(
        &imported.session.identity.id,
        &imported.session.events,
        TurnQuality::Inferred,
    );

    Ok(ProviderSessionImportPage {
        imported,
        event_count,
        message_count,
        turn_count: Some(turn_count),
        turns,
    })
}

pub(super) fn import_canonical_session_from_dir(session_dir: &Path) -> Result<ImportedSession> {
    let metadata = std::fs::symlink_metadata(session_dir).with_context(|| {
        format!(
            "Failed to read Kimi session directory: {}",
            session_dir.display()
        )
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kimi session source locator must be a directory: {}",
            session_dir.display()
        );
    }
    let context_path = session_dir.join("context.jsonl");
    if !context_path.is_file() {
        anyhow::bail!("Kimi context.jsonl not found: {}", context_path.display());
    }
    let wire_path = session_dir.join("wire.jsonl");
    let state_path = session_dir.join("state.json");
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let state_value = read_optional_kimi_state(&state_path, &mut report)?;
    let title = state_value
        .as_ref()
        .and_then(|state| state.get("custom_title"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string);
    let session_id = session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let project_dir = kimi_project_dir_for_session_dir(session_dir);
    let context_modified_at = kimi_file_modified_at(&context_path)?;

    let mut context_events =
        events_from_context(&context_path, context_modified_at, &mut report)?;
    let wire = events_from_wire(&wire_path, &mut report)?;
    reconcile_kimi_context_with_wire(&mut context_events, &wire.visible_events, &mut report);
    context_events.extend(wire.lifecycle_events);

    let mut extensions = BTreeMap::new();
    if let Some(state) = state_value {
        extensions.insert("kimi_state".to_string(), state);
    }
    if !wire.metadata_headers.is_empty() {
        extensions.insert(
            "kimi_wire_metadata".to_string(),
            Value::Array(wire.metadata_headers),
        );
    }
    if !wire.unsequenced_records.is_empty() {
        extensions.insert(
            "kimi_wire_unsequenced_records".to_string(),
            Value::Array(wire.unsequenced_records),
        );
    }

    let event_meta = context_events
        .iter()
        .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
        .collect::<Vec<_>>();
    Ok(ImportedSession {
        session: Session {
            schema: Schema::default(),
            identity: Identity {
                id: session_id.clone(),
                title,
            },
            context: Context {
                workspace: project_dir,
                created_at: wire.first_timestamp.or(Some(context_modified_at)),
                last_active_at: Some(context_modified_at),
                tags: Vec::new(),
            },
            events: context_events,
            extensions,
        },
        provenance: Provenance {
            imported_at: Utc::now(),
            imported_by: Some("memorph-cli".to_string()),
            primary_source: ProviderRef {
                provider_id: PROVIDER_ID.to_string(),
                session_id,
                source_path: Some(session_dir.to_string_lossy().to_string()),
            },
            aliases: Vec::new(),
        },
        event_meta,
        report,
    })
}

pub(super) fn read_optional_kimi_state(
    state_path: &Path,
    report: &mut MappingReport,
) -> Result<Option<Value>> {
    let raw = match std::fs::read_to_string(state_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to read Kimi state: {}", state_path.display()))
        }
    };
    match serde_json::from_str(&raw) {
        Ok(state) => Ok(Some(state)),
        Err(error) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: Fidelity::Dropped,
                code: "invalid_state_json".to_string(),
                message: format!("Failed to parse Kimi state.json: {error}"),
                path: Some("state.json".to_string()),
                raw: Some(Value::String(raw)),
            });
            Ok(None)
        }
    }
}

pub(super) fn kimi_file_modified_at(path: &Path) -> Result<chrono::DateTime<Utc>> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("Failed to read Kimi source metadata: {}", path.display()))?;
    let modified = metadata
        .modified()
        .with_context(|| format!("Failed to read Kimi source mtime: {}", path.display()))?;
    Ok(chrono::DateTime::<Utc>::from(modified))
}

#[derive(Default)]
pub(super) struct KimiWireImport {
    visible_events: Vec<Event>,
    lifecycle_events: Vec<Event>,
    metadata_headers: Vec<Value>,
    unsequenced_records: Vec<Value>,
    first_timestamp: Option<chrono::DateTime<Utc>>,
}

#[derive(Clone)]
pub(super) struct KimiWireTurn {
    provider_turn_id: String,
}

pub(super) fn events_from_wire(
    wire_path: &Path,
    report: &mut MappingReport,
) -> Result<KimiWireImport> {
    let file = match File::open(wire_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(KimiWireImport::default())
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("Failed to open Kimi wire.jsonl: {}", wire_path.display())
            })
        }
    };
    let reader = BufReader::new(file);
    let mut imported = KimiWireImport::default();
    let mut active_turn: Option<KimiWireTurn> = None;
    let mut assistant_blocks: Vec<Block> = Vec::new();
    let mut assistant_raw_parts: Vec<Value> = Vec::new();
    let mut assistant_timestamp: Option<chrono::DateTime<Utc>> = None;
    let mut assistant_line_number: Option<usize> = None;

    for (line_idx, line) in reader.lines().enumerate() {
        let line_number = line_idx + 1;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: Fidelity::Dropped,
                    code: "invalid_wire_jsonl_line".to_string(),
                    message: format!("Failed to parse Kimi wire line: {error}"),
                    path: Some(format!("wire.jsonl:line:{line_number}")),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };

        if value.get("type").and_then(Value::as_str) == Some("metadata")
            && value.get("message").is_none()
        {
            imported.metadata_headers.push(value);
            continue;
        }

        let Some(timestamp) = parse_wire_timestamp(&value) else {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: Fidelity::Downgraded,
                code: "wire_record_without_timestamp".to_string(),
                message: "Preserved Kimi wire record without inventing an event timestamp"
                    .to_string(),
                path: Some(format!("wire.jsonl:line:{line_number}")),
                raw: Some(value.clone()),
            });
            imported.unsequenced_records.push(value);
            continue;
        };
        imported.first_timestamp = imported.first_timestamp.or(Some(timestamp));

        let message_type = value
            .get("message")
            .and_then(|message| message.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        match message_type {
            "TurnBegin" => {
                flush_kimi_wire_assistant(
                    &mut imported.visible_events,
                    &mut assistant_blocks,
                    &mut assistant_raw_parts,
                    &mut assistant_timestamp,
                    &mut assistant_line_number,
                    active_turn.as_ref(),
                );
                let turn = KimiWireTurn {
                    provider_turn_id: format!("kimi:wire-turn:{line_number}"),
                };
                active_turn = Some(turn.clone());
                imported.lifecycle_events.push(kimi_wire_event(
                    format!("kimi:wire:TurnBegin:{line_number}"),
                    EventKind::Lifecycle,
                    Role::System,
                    timestamp,
                    Some((&turn, None)),
                    vec![Block::Other { raw: value.clone() }],
                    vec![value.clone()],
                ));
                let payload = value
                    .get("message")
                    .and_then(|message| message.get("payload"));
                let blocks = kimi_user_input_event_blocks(payload, &value, line_number, report);
                if !blocks.is_empty() {
                    imported.visible_events.push(kimi_wire_event(
                        format!("kimi:wire:user:{line_number}"),
                        EventKind::Message,
                        Role::User,
                        timestamp,
                        Some((&turn, None)),
                        blocks,
                        vec![value],
                    ));
                }
            }
            "ContentPart" => {
                let payload = value
                    .get("message")
                    .and_then(|message| message.get("payload"));
                if let Some(block) =
                    kimi_content_part_event_block(payload, &value, line_number, report)
                {
                    if matches!(block, Block::Other { .. }) {
                        imported.lifecycle_events.push(kimi_wire_event(
                            format!("kimi:wire:ContentPart:{line_number}"),
                            EventKind::Other,
                            Role::Assistant,
                            timestamp,
                            active_turn.as_ref().map(|turn| (turn, None)),
                            vec![block],
                            vec![value],
                        ));
                    } else {
                        assistant_blocks.push(block);
                        assistant_raw_parts.push(value);
                        assistant_timestamp = assistant_timestamp.or(Some(timestamp));
                        assistant_line_number = assistant_line_number.or(Some(line_number));
                    }
                }
            }
            "TurnEnd" => {
                flush_kimi_wire_assistant(
                    &mut imported.visible_events,
                    &mut assistant_blocks,
                    &mut assistant_raw_parts,
                    &mut assistant_timestamp,
                    &mut assistant_line_number,
                    active_turn.as_ref(),
                );
                imported.lifecycle_events.push(kimi_wire_event(
                    format!("kimi:wire:TurnEnd:{line_number}"),
                    EventKind::Lifecycle,
                    Role::System,
                    timestamp,
                    active_turn
                        .as_ref()
                        .map(|turn| (turn, Some(TurnOutcome::Completed))),
                    vec![Block::Other { raw: value.clone() }],
                    vec![value],
                ));
                active_turn = None;
            }
            "StepBegin" | "StatusUpdate" => {
                imported.lifecycle_events.push(kimi_wire_event(
                    format!("kimi:wire:{message_type}:{line_number}"),
                    EventKind::Lifecycle,
                    Role::System,
                    timestamp,
                    active_turn.as_ref().map(|turn| (turn, None)),
                    vec![Block::Other { raw: value.clone() }],
                    vec![value],
                ));
            }
            other => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: Fidelity::Preserved,
                    code: "provider_wire_message_preserved".to_string(),
                    message: format!("Preserved unsupported Kimi wire message '{other}'"),
                    path: Some(format!("wire.jsonl:line:{line_number}")),
                    raw: Some(value.clone()),
                });
                imported.lifecycle_events.push(kimi_wire_event(
                    format!("kimi:wire:{other}:{line_number}"),
                    EventKind::Other,
                    Role::System,
                    timestamp,
                    active_turn.as_ref().map(|turn| (turn, None)),
                    vec![Block::Other { raw: value.clone() }],
                    vec![value],
                ));
            }
        }
    }

    flush_kimi_wire_assistant(
        &mut imported.visible_events,
        &mut assistant_blocks,
        &mut assistant_raw_parts,
        &mut assistant_timestamp,
        &mut assistant_line_number,
        active_turn.as_ref(),
    );
    Ok(imported)
}

pub(super) fn flush_kimi_wire_assistant(
    events: &mut Vec<Event>,
    assistant_blocks: &mut Vec<Block>,
    assistant_raw_parts: &mut Vec<Value>,
    assistant_timestamp: &mut Option<chrono::DateTime<Utc>>,
    assistant_line_number: &mut Option<usize>,
    turn: Option<&KimiWireTurn>,
) {
    if assistant_blocks.is_empty() {
        assistant_raw_parts.clear();
        *assistant_timestamp = None;
        *assistant_line_number = None;
        return;
    }
    let blocks = std::mem::take(assistant_blocks);
    let raw_parts = std::mem::take(assistant_raw_parts);
    let timestamp = assistant_timestamp
        .take()
        .expect("Kimi assistant content has a timestamp");
    let line_number = assistant_line_number
        .take()
        .expect("Kimi assistant content has a line number");
    events.push(kimi_wire_event(
        format!("kimi:wire:assistant:{line_number}"),
        kimi_event_kind(&blocks),
        Role::Assistant,
        timestamp,
        turn.map(|turn| (turn, None)),
        blocks,
        raw_parts,
    ));
}

pub(super) fn events_from_context(
    context_path: &Path,
    fallback_timestamp: chrono::DateTime<Utc>,
    report: &mut MappingReport,
) -> Result<Vec<Event>> {
    let file = File::open(context_path).with_context(|| {
        format!(
            "Failed to open Kimi context.jsonl: {}",
            context_path.display()
        )
    })?;
    let mut events = Vec::new();
    for (line_idx, line) in BufReader::new(file).lines().enumerate() {
        let line_number = line_idx + 1;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: Fidelity::Dropped,
                    code: "invalid_context_jsonl_line".to_string(),
                    message: format!("Failed to parse Kimi context line: {error}"),
                    path: Some(format!("context.jsonl:line:{line_number}")),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };
        events.push(kimi_context_event(
            value,
            line_number,
            fallback_timestamp,
            report,
        ));
    }
    Ok(events)
}

pub(super) fn kimi_context_event(
    value: Value,
    line_number: usize,
    timestamp: chrono::DateTime<Utc>,
    report: &mut MappingReport,
) -> Event {
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .map(str::to_string);
    let (kind, event_role, blocks) = match role.as_deref() {
        Some("_system_prompt") => (
            EventKind::Message,
            Role::System,
            kimi_context_content_blocks(value.get("content"), &value, line_number, report),
        ),
        Some("user") => (
            EventKind::Message,
            Role::User,
            kimi_context_content_blocks(value.get("content"), &value, line_number, report),
        ),
        Some("assistant") => {
            let blocks =
                kimi_context_content_blocks(value.get("content"), &value, line_number, report);
            (kimi_event_kind(&blocks), Role::Assistant, blocks)
        }
        Some("_checkpoint" | "_usage") => (
            EventKind::Lifecycle,
            Role::System,
            vec![Block::Other { raw: value.clone() }],
        ),
        Some(other) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: Fidelity::Preserved,
                code: "provider_context_role_preserved".to_string(),
                message: format!("Preserved unsupported Kimi context role '{other}'"),
                path: Some(format!("context.jsonl:line:{line_number}")),
                raw: Some(value.clone()),
            });
            (
                EventKind::Other,
                Role::Other,
                vec![Block::Other { raw: value.clone() }],
            )
        }
        None => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: Fidelity::Downgraded,
                code: "context_record_without_role".to_string(),
                message: "Preserved Kimi context record without a role".to_string(),
                path: Some(format!("context.jsonl:line:{line_number}")),
                raw: Some(value.clone()),
            });
            (
                EventKind::Other,
                Role::Other,
                vec![Block::Other { raw: value.clone() }],
            )
        }
    };

    Event {
        id: format!("kimi:context:{line_number}"),
        kind,
        role: event_role,
        timestamp,
        links: Links::default(),
        blocks,
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: None,
            usage: None,
        },
    }
}

pub(super) fn kimi_context_content_blocks(
    content: Option<&Value>,
    raw_line: &Value,
    line_number: usize,
    report: &mut MappingReport,
) -> Vec<Block> {
    match content {
        Some(Value::String(text)) => vec![Block::Text { text: text.clone() }],
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                kimi_context_content_block(item, raw_line, line_number, index, report)
            })
            .collect(),
        Some(value) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: Fidelity::Preserved,
                code: "provider_context_content_preserved".to_string(),
                message: "Preserved non-string Kimi context content".to_string(),
                path: Some(format!("context.jsonl:line:{line_number}:content")),
                raw: Some(raw_line.clone()),
            });
            vec![Block::Other { raw: value.clone() }]
        }
        None => Vec::new(),
    }
}

pub(super) fn kimi_context_content_block(
    item: &Value,
    raw_line: &Value,
    line_number: usize,
    item_index: usize,
    report: &mut MappingReport,
) -> Block {
    if let Some(text) = item.as_str() {
        return Block::Text {
            text: text.to_string(),
        };
    }
    match item.get("type").and_then(Value::as_str) {
        Some("text") => Block::Text {
            text: item
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
        Some("think") => Block::Thinking {
            text: item
                .get("think")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            signature: item
                .get("encrypted")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        Some("image_url") => Block::Image {
            mime_type: "image/png".to_string(),
            data: item
                .get("image_url")
                .and_then(|image| image.get("url"))
                .and_then(Value::as_str)
                .map(str::to_string),
            path: None,
        },
        Some(kind) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: Fidelity::Preserved,
                code: "provider_context_block_preserved".to_string(),
                message: format!("Preserved unsupported Kimi context block '{kind}'"),
                path: Some(format!(
                    "context.jsonl:line:{line_number}:content:{item_index}"
                )),
                raw: Some(raw_line.clone()),
            });
            Block::Other { raw: item.clone() }
        }
        None => Block::Other { raw: item.clone() },
    }
}

pub(super) fn reconcile_kimi_context_with_wire(
    context_events: &mut [Event],
    wire_events: &[Event],
    report: &mut MappingReport,
) {
    let mut used = vec![false; wire_events.len()];
    let mut cursor = 0usize;
    for context_event in context_events {
        let Some(context_text) = event_visible_message_text(context_event) else {
            continue;
        };
        let Some((wire_index, wire_event)) =
            wire_events
                .iter()
                .enumerate()
                .skip(cursor)
                .find(|(_, wire_event)| {
                    wire_event.role == context_event.role
                        && event_visible_message_text(wire_event)
                            .is_some_and(|wire_text| wire_text.trim() == context_text.trim())
                })
        else {
            continue;
        };
        used[wire_index] = true;
        cursor = wire_index + 1;
        context_event.timestamp = wire_event.timestamp;
        context_event.links = wire_event.links.clone();
        for block in &wire_event.blocks {
            if matches!(block, Block::Other { .. })
                && !context_event.blocks.iter().any(|existing| {
                    serde_json::to_value(existing).ok() == serde_json::to_value(block).ok()
                })
            {
                context_event.blocks.push(block.clone());
            }
        }
    }

    for (wire_index, wire_event) in wire_events.iter().enumerate() {
        if used[wire_index] {
            continue;
        }
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: Fidelity::Dropped,
            code: "wire_content_not_in_context".to_string(),
            message:
                "Dropped Kimi wire content that was not present in authoritative context.jsonl"
                    .to_string(),
            path: Some(wire_event.id.clone()),
            raw: None,
        });
    }
}

pub(super) fn kimi_wire_event(
    id: String,
    kind: EventKind,
    role: Role,
    timestamp: chrono::DateTime<Utc>,
    turn: Option<(&KimiWireTurn, Option<TurnOutcome>)>,
    blocks: Vec<Block>,
    _raw_parts: Vec<Value>,
) -> Event {
    Event {
        id: id.clone(),
        kind,
        role,
        timestamp,
        links: Links {
            parent_event_id: None,
            turn_id: turn.map(|(turn, _)| turn.provider_turn_id.clone()),
            turn_outcome: turn.and_then(|(_, boundary)| boundary),
            related_event_ids: Vec::new(),
        },
        blocks,
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: None,
            usage: None,
        },
    }
}

pub(super) fn kimi_user_input_event_blocks(
    payload: Option<&Value>,
    raw_line: &Value,
    line_number: usize,
    report: &mut MappingReport,
) -> Vec<Block> {
    let Some(inputs) = payload
        .and_then(|payload| payload.get("user_input"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    inputs
        .iter()
        .enumerate()
        .map(
            |(idx, item)| match item.get("type").and_then(Value::as_str) {
                Some("text") => Block::Text {
                    text: item
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                },
                Some("image_url") => Block::Image {
                    mime_type: "image/png".to_string(),
                    data: item
                        .get("image_url")
                        .and_then(|value| value.get("url"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    path: None,
                },
                Some(kind) => {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Info,
                        disposition: Fidelity::Preserved,
                        code: "provider_block_preserved".to_string(),
                        message: format!("Preserved unsupported Kimi user input '{kind}'"),
                        path: Some(format!("wire.jsonl:line:{line_number}:input:{idx}")),
                        raw: Some(raw_line.clone()),
                    });
                    Block::Other { raw: item.clone() }
                }
                None => Block::Other { raw: item.clone() },
            },
        )
        .collect()
}

pub(super) fn kimi_content_part_event_block(
    payload: Option<&Value>,
    raw_line: &Value,
    line_number: usize,
    report: &mut MappingReport,
) -> Option<Block> {
    let payload = payload?;
    match payload.get("type").and_then(Value::as_str) {
        Some("text") => Some(Block::Text {
            text: payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
        Some("think") => Some(Block::Thinking {
            text: payload
                .get("think")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            signature: payload
                .get("encrypted")
                .and_then(Value::as_str)
                .map(str::to_string),
        }),
        Some(kind) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: Fidelity::Preserved,
                code: "provider_block_preserved".to_string(),
                message: format!("Preserved unsupported Kimi content part '{kind}'"),
                path: Some(format!("wire.jsonl:line:{line_number}")),
                raw: Some(raw_line.clone()),
            });
            Some(Block::Other {
                raw: payload.clone(),
            })
        }
        None => Some(Block::Other {
            raw: payload.clone(),
        }),
    }
}

pub(super) fn kimi_event_kind(blocks: &[Block]) -> EventKind {
    if blocks
        .iter()
        .any(|block| matches!(block, Block::ToolResult { .. }))
    {
        EventKind::Observation
    } else if blocks
        .iter()
        .any(|block| matches!(block, Block::ToolCall { .. }))
    {
        EventKind::Action
    } else if blocks
        .iter()
        .all(|block| matches!(block, Block::Other { .. }))
    {
        EventKind::Other
    } else {
        EventKind::Message
    }
}

pub(super) fn kimi_project_dir_for_session_dir(session_dir: &Path) -> Option<String> {
    let project_hash = session_dir
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())?;
    load_work_dir_map()
        .ok()?
        .get(project_hash)
        .map(|work_dir| work_dir.project_dir.clone())
}

pub(super) fn parse_wire_timestamp(value: &Value) -> Option<chrono::DateTime<Utc>> {
    let ts = value.get("timestamp").and_then(|v| v.as_f64())?;
    let secs = ts as i64;
    let nanos = ((ts - secs as f64) * 1e9).max(0.0) as u32;
    chrono::DateTime::from_timestamp(secs, nanos)
}
