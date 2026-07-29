use super::*;

pub(super) fn import_canonical_session(path: &Path) -> Result<ImportedSession> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex session: {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::new();
    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<chrono::DateTime<Utc>> = None;
    let mut last_active_at: Option<chrono::DateTime<Utc>> = None;
    let mut source_title: Option<String> = None;
    let mut extensions = BTreeMap::new();
    let mut turn_tracker = CodexTurnTracker::default();

    for (line_idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: Fidelity::Dropped,
                    code: "invalid_jsonl_line".to_string(),
                    message: format!("Failed to parse Codex session line: {}", error),
                    path: Some(format!("line:{}", line_idx + 1)),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };

        let line_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let timestamp = codex_line_timestamp(&value, line_idx + 1);
        last_active_at = Some(timestamp);
        let first_new_event = events.len();
        let turn_link = turn_tracker.observe_line(&value);

        match line_type.as_str() {
            "session_meta" => {
                if let Some(payload) = value.get("payload") {
                    session_id = payload
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(session_id);
                    project_dir = payload
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(project_dir);
                    created_at = payload
                        .get("timestamp")
                        .and_then(|v| v.as_str())
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&Utc))
                        .or(created_at);

                    if let Some(text) = payload
                        .get("base_instructions")
                        .and_then(|v| v.get("text"))
                        .and_then(|v| v.as_str())
                    {
                        events.push(Event {
                            id: format!("codex:base_instructions:{}", line_idx + 1),
                            kind: EventKind::Lifecycle,
                            role: Role::System,
                            timestamp,
                            links: Links::default(),
                            blocks: vec![
                                Block::Text {
                                    text: text.to_string(),
                                },
                                Block::Other {
                                    raw: payload.clone(),
                                },
                            ],
                            metadata: Metadata {
                                model: payload
                                    .get("model")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string),
                                usage: None,
                            },
                        });
                    } else {
                        events.push(provider_payload_event(
                            format!("codex:session_meta:{}", line_idx + 1),
                            EventKind::Lifecycle,
                            Role::System,
                            timestamp,
                            "session_meta",
                            ProviderPayloadData {
                                payload: payload.clone(),
                            },
                        ));
                    }

                    source_title = payload
                        .get("title")
                        .or_else(|| payload.get("thread_name"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(source_title);
                    extensions.insert("codex_session_meta".to_string(), payload.clone());
                }
            }
            "turn_context" => {
                if let Some(payload) = value.get("payload") {
                    project_dir = payload
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(project_dir);
                    events.push(provider_payload_event(
                        format!("codex:turn_context:{}", line_idx + 1),
                        EventKind::Lifecycle,
                        Role::System,
                        timestamp,
                        "turn_context",
                        ProviderPayloadData {
                            payload: payload.clone(),
                        },
                    ));
                }
            }
            "event_msg" => {
                if let Some(payload) = value.get("payload") {
                    events.push(codex_event_msg_event(
                        payload,
                        timestamp,
                        line_idx + 1,
                        value.clone(),
                    ));
                }
            }
            "response_item" => {
                if let Some(payload) = value.get("payload") {
                    let msg_type = payload.get("type").and_then(|v| v.as_str());
                    if msg_type == Some("token_count") {
                        continue;
                    }
                    events.push(codex_response_item_event(
                        payload,
                        timestamp,
                        line_idx + 1,
                        value.clone(),
                        &mut report,
                    ));
                }
            }
            "compacted" => {
                if let Some(payload) = value.get("payload") {
                    events.push(codex_compacted_event(
                        payload,
                        timestamp,
                        line_idx + 1,
                        value.clone(),
                    ));
                }
            }
            other => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: Fidelity::Normalized,
                    code: "unknown_codex_line".to_string(),
                    message: format!("Preserved unknown Codex line type '{}'", other),
                    path: Some(format!("line:{}", line_idx + 1)),
                    raw: Some(value.clone()),
                });
                events.push(provider_payload_event(
                    format!("codex:unknown:{}", line_idx + 1),
                    EventKind::Other,
                    Role::Other,
                    timestamp,
                    other,
                    ProviderPayloadData {
                        payload: value.get("payload").cloned().unwrap_or(Value::Null),
                    },
                ));
            }
        }
        for event in &mut events[first_new_event..] {
            turn_link.clone().apply_to(event);
        }
    }

    let canonical_id = session_id
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let source_title =
        select_codex_display_title(None, None, source_title.as_deref(), &canonical_id);

    let event_meta = events
        .iter()
        .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
        .collect::<Vec<_>>();
    Ok(ImportedSession {
        session: Session {
            schema: Schema::default(),
            identity: Identity {
                id: canonical_id.clone(),
                title: source_title,
            },
            context: Context {
                workspace: project_dir,
                created_at,
                last_active_at,
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
                session_id: canonical_id,
                source_path: Some(path.to_string_lossy().to_string()),
            },
            aliases: Vec::new(),
        },
        event_meta,
        report,
    })
}

pub(super) fn codex_line_timestamp(value: &Value, line_number: usize) -> chrono::DateTime<Utc> {
    value
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| {
            chrono::DateTime::from_timestamp_millis(line_number as i64)
                .expect("Codex source line number is a valid timestamp")
        })
}

pub(super) fn import_canonical_session_page(
    path: &Path,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<ProviderSessionImportPage> {
    let (state, locations) = load_or_build_codex_event_index_page(path, event_offset, event_limit)?;

    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::with_capacity(locations.len());
    let mut file = File::open(path)
        .with_context(|| format!("Failed to open Codex session: {}", path.display()))?;

    for location in locations {
        file.seek(SeekFrom::Start(location.byte_offset))
            .with_context(|| format!("Failed to seek Codex session: {}", path.display()))?;
        let mut line_bytes = vec![0u8; location.byte_length as usize];
        file.read_exact(&mut line_bytes)
            .with_context(|| format!("Failed to read Codex session: {}", path.display()))?;
        let line = String::from_utf8_lossy(&line_bytes);
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: Fidelity::Dropped,
                    code: "invalid_jsonl_line".to_string(),
                    message: format!("Failed to parse Codex session line: {}", error),
                    path: Some(format!("line:{}", location.line_no)),
                    raw: Some(Value::String(line.into_owned())),
                });
                continue;
            }
        };

        let line_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let payload = value.get("payload");
        let timestamp = codex_line_timestamp(&value, location.line_no);

        if let Some(mut event) = codex_event_from_line(
            line_type,
            payload,
            timestamp,
            location.line_no,
            value.clone(),
            &mut report,
        ) {
            CodexTurnLink {
                turn_id: location.provider_turn_id,
                turn_outcome: location.turn_boundary,
            }
            .apply_to(&mut event);
            events.push(event);
        }
    }

    let event_meta = events
        .iter()
        .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
        .collect::<Vec<_>>();
    let imported = ImportedSession {
        session: Session {
            schema: Schema::default(),
            identity: Identity {
                id: state.session_id.clone(),
                title: state.source_title.clone(),
            },
            context: Context {
                workspace: state.workspace_dir.clone(),
                created_at: state
                    .created_at_ms
                    .and_then(chrono::DateTime::from_timestamp_millis),
                last_active_at: state
                    .last_active_at_ms
                    .and_then(chrono::DateTime::from_timestamp_millis),
                tags: Vec::new(),
            },
            events,
            extensions: BTreeMap::new(),
        },
        provenance: Provenance {
            imported_at: Utc::now(),
            imported_by: Some("memorph-cli".to_string()),
            primary_source: ProviderRef {
                provider_id: PROVIDER_ID.to_string(),
                session_id: state.session_id.clone(),
                source_path: Some(path.to_string_lossy().to_string()),
            },
            aliases: Vec::new(),
        },
        event_meta,
        report,
    };

    let turns = crate::session_projection::project_session_turns(
        &imported.session.identity.id,
        &imported.session.events,
        TurnQuality::Exact,
    );
    let turn_count = (event_offset == 0 && imported.session.events.len() == state.event_count)
        .then_some(turns.len());
    Ok(ProviderSessionImportPage {
        imported,
        event_count: state.event_count,
        message_count: state.message_count,
        turn_count,
        turns,
    })
}

pub(super) fn load_or_build_codex_event_index_page(
    path: &Path,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<(
    event_index::IndexedSessionState,
    Vec<event_index::IndexedEventLocation>,
)> {
    let source_path = path.to_string_lossy().to_string();
    let fingerprint = event_index::source_file_fingerprint(path)?;
    let mut conn = event_index::open_database()?;
    let mut state =
        match event_index::load_fresh_session_state(&conn, PROVIDER_ID, &source_path, fingerprint)?
        {
            Some(state) => state,
            None => {
                let (state, locations) = build_codex_event_index(path, fingerprint)?;
                event_index::replace_session_index(&mut conn, &state, &locations)?;
                state
            }
        };
    let mut locations = event_index::load_event_locations(
        &conn,
        PROVIDER_ID,
        &source_path,
        fingerprint,
        event_offset,
        event_limit,
    )?;

    if locations.is_empty() && event_offset < state.event_count && event_limit != Some(0) {
        let (rebuilt_state, rebuilt_locations) = build_codex_event_index(path, fingerprint)?;
        event_index::replace_session_index(&mut conn, &rebuilt_state, &rebuilt_locations)?;
        state = rebuilt_state;
        locations = event_index::load_event_locations(
            &conn,
            PROVIDER_ID,
            &source_path,
            fingerprint,
            event_offset,
            event_limit,
        )?;
    }

    Ok((state, locations))
}

pub(super) fn build_codex_event_index(
    path: &Path,
    fingerprint: event_index::SourceFileFingerprint,
) -> Result<(
    event_index::IndexedSessionState,
    Vec<event_index::IndexedEventLocation>,
)> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex session: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut byte_offset = 0u64;
    let mut line_no = 0usize;
    let mut event_count = 0usize;
    let mut message_count = 0usize;
    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at_ms: Option<i64> = None;
    let mut last_active_at_ms: Option<i64> = None;
    let mut source_title: Option<String> = None;
    let mut locations = Vec::new();
    let mut turn_tracker = CodexTurnTracker::default();

    loop {
        line.clear();
        let byte_length = reader
            .read_line(&mut line)
            .with_context(|| format!("Failed to read Codex session: {}", path.display()))?;
        if byte_length == 0 {
            break;
        }
        line_no += 1;
        let line_offset = byte_offset;
        byte_offset += byte_length as u64;

        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let line_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let payload = value.get("payload");
        let timestamp = codex_line_timestamp(&value, line_no);
        last_active_at_ms = Some(timestamp.timestamp_millis());
        let turn_link = turn_tracker.observe_line(&value);

        if line_type == "session_meta" {
            if let Some(payload) = payload {
                session_id = payload
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or(session_id);
                project_dir = payload
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or(project_dir);
                created_at_ms = payload
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.timestamp_millis())
                    .or(created_at_ms);
                source_title = payload
                    .get("title")
                    .or_else(|| payload.get("thread_name"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or(source_title);
            }
        } else if line_type == "turn_context" {
            if let Some(payload) = payload {
                project_dir = payload
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or(project_dir);
            }
        }

        if !codex_line_produces_event(line_type, payload) {
            continue;
        }

        if codex_line_is_visible_message(line_type, payload) {
            message_count += 1;
        }

        locations.push(event_index::IndexedEventLocation {
            event_index: event_count,
            byte_offset: line_offset,
            byte_length: byte_length as u64,
            line_no,
            provider_turn_id: turn_link.turn_id,
            turn_index: None,
            turn_boundary: turn_link.turn_outcome,
        });
        event_count += 1;
    }

    let session_id = session_id
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let source_title = select_codex_display_title(None, None, source_title.as_deref(), &session_id);

    Ok((
        event_index::IndexedSessionState {
            provider_id: PROVIDER_ID.to_string(),
            session_id,
            source_path: path.to_string_lossy().to_string(),
            file_fingerprint: fingerprint,
            workspace_dir: project_dir,
            created_at_ms,
            last_active_at_ms,
            source_title,
            event_count,
            message_count,
        },
        locations,
    ))
}

pub(super) fn codex_line_produces_event(line_type: &str, payload: Option<&Value>) -> bool {
    match line_type {
        "session_meta" | "turn_context" | "event_msg" | "compacted" => payload.is_some(),
        "response_item" => {
            payload.is_some()
                && payload.and_then(|p| p.get("type")).and_then(Value::as_str)
                    != Some("token_count")
        }
        _ => true,
    }
}

pub(super) fn codex_line_is_visible_message(line_type: &str, payload: Option<&Value>) -> bool {
    match line_type {
        "response_item" => {
            let Some(payload) = payload else {
                return false;
            };
            match payload.get("type").and_then(Value::as_str) {
                Some("function_call" | "function_call_output") => true,
                Some("message") => {
                    let role = payload.get("role").and_then(Value::as_str);
                    if !matches!(role, Some("user" | "assistant" | "tool")) {
                        return false;
                    }
                    codex_response_message_has_visible_content(payload)
                }
                _ => false,
            }
        }
        "compacted" => true,
        _ => false,
    }
}

pub(super) fn codex_response_message_has_visible_content(payload: &Value) -> bool {
    if payload
        .get("content")
        .and_then(Value::as_str)
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    payload
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks.iter().any(|block| {
                matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("input_text" | "output_text" | "refusal" | "input_image")
                )
            })
        })
        .unwrap_or(false)
}

pub(super) fn codex_event_from_line(
    line_type: &str,
    payload: Option<&Value>,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    raw_line: Value,
    report: &mut MappingReport,
) -> Option<Event> {
    match line_type {
        "session_meta" => payload.map(|payload| {
            if let Some(text) = payload
                .get("base_instructions")
                .and_then(|v| v.get("text"))
                .and_then(Value::as_str)
            {
                Event {
                    id: format!("codex:base_instructions:{}", line_no),
                    kind: EventKind::Lifecycle,
                    role: Role::System,
                    timestamp,
                    links: Links::default(),
                    blocks: vec![
                        Block::Text {
                            text: text.to_string(),
                        },
                        Block::Other {
                            raw: payload.clone(),
                        },
                    ],
                    metadata: Metadata {
                        model: payload
                            .get("model")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        usage: None,
                    },
                }
            } else {
                provider_payload_event(
                    format!("codex:session_meta:{}", line_no),
                    EventKind::Lifecycle,
                    Role::System,
                    timestamp,
                    "session_meta",
                    ProviderPayloadData {
                        payload: payload.clone(),
                    },
                )
            }
        }),
        "turn_context" => payload.map(|payload| {
            provider_payload_event(
                format!("codex:turn_context:{}", line_no),
                EventKind::Lifecycle,
                Role::System,
                timestamp,
                "turn_context",
                ProviderPayloadData {
                    payload: payload.clone(),
                },
            )
        }),
        "event_msg" => {
            payload.map(|payload| codex_event_msg_event(payload, timestamp, line_no, raw_line))
        }
        "response_item" => {
            let payload = payload?;
            if payload.get("type").and_then(Value::as_str) == Some("token_count") {
                None
            } else {
                Some(codex_response_item_event(
                    payload, timestamp, line_no, raw_line, report,
                ))
            }
        }
        "compacted" => {
            payload.map(|payload| codex_compacted_event(payload, timestamp, line_no, raw_line))
        }
        other => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: Fidelity::Normalized,
                code: "unknown_codex_line".to_string(),
                message: format!("Preserved unknown Codex line type '{}'", other),
                path: Some(format!("line:{}", line_no)),
                raw: Some(raw_line.clone()),
            });
            Some(provider_payload_event(
                format!("codex:unknown:{}", line_no),
                EventKind::Other,
                Role::Other,
                timestamp,
                other,
                ProviderPayloadData {
                    payload: raw_line.get("payload").cloned().unwrap_or(Value::Null),
                },
            ))
        }
    }
}

pub(super) fn codex_compacted_event(
    payload: &Value,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    _raw_line: Value,
) -> Event {
    let memorph = payload.get("memorph").and_then(Value::as_object);
    let source_provider_id = memorph
        .and_then(|value| value.get("source_provider_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(PROVIDER_ID)
        .to_string();
    let summary = memorph
        .and_then(|value| value.get("summary"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("message").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let source_event_ids = memorph
        .and_then(|value| value.get("source_event_ids"))
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
        .and_then(|value| value.get("source_event_count"))
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .or_else(|| (!source_event_ids.is_empty()).then_some(source_event_ids.len()));
    let archive_ref = memorph
        .and_then(|value| value.get("archive_ref"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    Event {
        id: format!("codex:compacted:{}", line_no),
        kind: EventKind::Message,
        role: Role::Assistant,
        timestamp,
        links: Links::default(),
        blocks: vec![Block::Compressed {
            raw: serde_json::json!({
                "format": "memorph.compressed.v1",
                "source_provider_id": source_provider_id,
                "summary": summary,
                "source_event_ids": source_event_ids,
                "source_event_count": source_event_count,
                "archive_ref": archive_ref,
            }),
        }],
        metadata: Metadata {
            model: None,
            usage: None,
        },
    }
}

pub(super) fn codex_response_item_event(
    payload: &Value,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    _raw_line: Value,
    report: &mut MappingReport,
) -> Event {
    let role_str = payload.get("role").and_then(|v| v.as_str());
    let msg_type = payload.get("type").and_then(|v| v.as_str());
    let phase = payload
        .get("phase")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let event_id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("codex:response_item:{}", line_no));

    if msg_type == Some("function_call") {
        let name = payload
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let call_id = payload
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let input = payload.get("arguments").cloned();
        let role = match role_str {
            Some("assistant") | None => Role::Assistant,
            _ => Role::Other,
        };
        return Event {
            id: event_id,
            kind: EventKind::Action,
            role,
            timestamp,
            links: Links::default(),
            blocks: vec![Block::ToolCall {
                tool_call_id: call_id.to_string(),
                name: name.to_string(),
                input,
            }],
            metadata: Metadata {
                model: None,
                usage: None,
            },
        };
    }

    if msg_type == Some("function_call_output") {
        let call_id = payload
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content = payload
            .get("output")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_default();
        return Event {
            id: event_id,
            kind: EventKind::Observation,
            role: Role::Tool,
            timestamp,
            links: Links::default(),
            blocks: vec![Block::ToolResult {
                tool_call_id: call_id.to_string(),
                content,
                is_error: false,
            }],
            metadata: Metadata {
                model: None,
                usage: None,
            },
        };
    }

    if msg_type != Some("message") {
        return provider_payload_event(
            event_id,
            EventKind::Other,
            Role::Other,
            timestamp,
            msg_type.unwrap_or("response_item"),
            ProviderPayloadData {
                payload: payload.clone(),
            },
        );
    }

    let mut blocks = Vec::new();
    if let Some(content_arr) = payload.get("content").and_then(|v| v.as_array()) {
        for (idx, block) in content_arr.iter().enumerate() {
            let Some(block_type) = block.get("type").and_then(|v| v.as_str()) else {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: Fidelity::Normalized,
                    code: "codex_block_missing_type".to_string(),
                    message: "Codex content block without a type was preserved as unknown"
                        .to_string(),
                    path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                    raw: Some(block.clone()),
                });
                blocks.push(Block::Other { raw: block.clone() });
                continue;
            };
            match block_type {
                "input_text" | "output_text" => {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        blocks.push(Block::Text {
                            text: text.to_string(),
                        });
                    } else {
                        report.push_issue(MappingIssue {
                            level: MappingIssueLevel::Warning,
                            disposition: Fidelity::Normalized,
                            code: "codex_text_block_missing_text".to_string(),
                            message:
                                "Codex text block without text was preserved as provider payload"
                                    .to_string(),
                            path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                            raw: Some(block.clone()),
                        });
                        blocks.push(Block::Other { raw: block.clone() });
                    }
                }
                "refusal" => {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        blocks.push(Block::Text {
                            text: text.to_string(),
                        });
                    } else {
                        report.push_issue(MappingIssue {
                            level: MappingIssueLevel::Warning,
                            disposition: Fidelity::Normalized,
                            code: "codex_refusal_block_missing_text".to_string(),
                            message:
                                "Codex refusal block without text was preserved as provider payload"
                                    .to_string(),
                            path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                            raw: Some(block.clone()),
                        });
                        blocks.push(Block::Other { raw: block.clone() });
                    }
                }
                "input_image" => {
                    if let Some(image_block) = codex_image_block(block) {
                        blocks.push(image_block);
                    } else {
                        report.push_issue(MappingIssue {
                            level: MappingIssueLevel::Info,
                            disposition: Fidelity::Normalized,
                            code: "codex_input_image_preserved_raw".to_string(),
                            message: "Codex input_image block was preserved as provider payload"
                                .to_string(),
                            path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                            raw: Some(block.clone()),
                        });
                        blocks.push(Block::Other { raw: block.clone() });
                    }
                }
                "reasoning" => {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Info,
                        disposition: Fidelity::Normalized,
                        code: "codex_reasoning_preserved_as_provider_payload".to_string(),
                        message: "Codex reasoning block was preserved as provider payload instead of being exposed as user-visible thinking"
                            .to_string(),
                        path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                        raw: Some(block.clone()),
                    });
                    blocks.push(Block::Other { raw: block.clone() });
                }
                other => {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Info,
                        disposition: Fidelity::Normalized,
                        code: "codex_unknown_block_preserved".to_string(),
                        message: format!("Preserved unknown Codex content block '{}'", other),
                        path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                        raw: Some(block.clone()),
                    });
                    blocks.push(Block::Other { raw: block.clone() });
                }
            }
        }
    } else if let Some(text) = payload.get("content").and_then(|v| v.as_str()) {
        blocks.push(Block::Text {
            text: text.to_string(),
        });
    } else {
        blocks.push(Block::Other {
            raw: payload.clone(),
        });
    }

    if blocks.is_empty() {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: Fidelity::Normalized,
            code: "codex_message_without_mappable_blocks".to_string(),
            message:
                "Codex message had no mappable content blocks and was preserved as provider payload"
                    .to_string(),
            path: Some(format!("response_item:{}", line_no)),
            raw: Some(payload.clone()),
        });
        blocks.push(Block::Other {
            raw: payload.clone(),
        });
    }

    if phase.as_deref() == Some("commentary") && blocks.len() == 1 {
        if let Block::Text { text } = &blocks[0] {
            blocks[0] = Block::Thinking {
                text: text.clone(),
                signature: None,
            };
        }
    }

    let role = match role_str {
        Some("user") => Role::User,
        Some("assistant") => Role::Assistant,
        Some("developer") => Role::Developer,
        Some("system") => Role::System,
        Some("tool") => Role::Tool,
        _ => Role::Other,
    };

    if let Some(internal_kind) = codex_internal_message_kind(role_str, &blocks) {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: Fidelity::Normalized,
            code: internal_kind.issue_code().to_string(),
            message: internal_kind.issue_message().to_string(),
            path: Some(format!("response_item:{}", line_no)),
            raw: Some(payload.clone()),
        });
        return codex_hidden_response_item_event(
            event_id,
            timestamp,
            internal_kind,
            payload.clone(),
            role_str,
        );
    }

    Event {
        id: event_id,
        kind: EventKind::Message,
        role,
        timestamp,
        links: Links::default(),
        blocks,
        metadata: Metadata {
            model: None,
            usage: None,
        },
    }
}

pub(super) fn codex_hidden_response_item_event(
    id: String,
    timestamp: chrono::DateTime<Utc>,
    internal_kind: CodexInternalMessageKind,
    payload: Value,
    _original_role: Option<&str>,
) -> Event {
    provider_payload_event(
        id,
        EventKind::Lifecycle,
        Role::System,
        timestamp,
        internal_kind.payload_kind(),
        ProviderPayloadData { payload },
    )
}

pub(super) fn codex_internal_message_kind(
    role_str: Option<&str>,
    blocks: &[Block],
) -> Option<CodexInternalMessageKind> {
    if codex_is_turn_aborted_sentinel(role_str, blocks) {
        return Some(CodexInternalMessageKind::LifecycleSentinel);
    }
    if codex_is_internal_user_context_message(role_str, blocks) {
        return Some(CodexInternalMessageKind::RuntimeContext);
    }
    if codex_is_internal_developer_control_message(role_str, blocks) {
        return Some(CodexInternalMessageKind::ProviderControl);
    }
    None
}

pub(super) fn codex_text_blocks(blocks: &[Block]) -> impl Iterator<Item = &str> {
    blocks.iter().filter_map(|block| match block {
        Block::Text { text } => Some(text.as_str()),
        _ => None,
    })
}

pub(super) fn codex_is_turn_aborted_sentinel(role_str: Option<&str>, blocks: &[Block]) -> bool {
    if role_str != Some("user") {
        return false;
    }
    let mut text_blocks = codex_text_blocks(blocks);
    let Some(text) = text_blocks.next() else {
        return false;
    };
    text_blocks.next().is_none()
        && text.trim_start().starts_with("<turn_aborted>")
        && text.trim_end().ends_with("</turn_aborted>")
}

pub(super) fn codex_is_internal_developer_control_message(
    role_str: Option<&str>,
    blocks: &[Block],
) -> bool {
    if role_str != Some("developer") {
        return false;
    }
    let mut saw_text = false;
    for text in codex_text_blocks(blocks) {
        saw_text = true;
        if !CODEX_INTERNAL_DEVELOPER_TAGS
            .iter()
            .any(|tag| text.trim_start().starts_with(tag))
        {
            return false;
        }
    }
    saw_text
}

pub(super) fn codex_is_internal_user_context_message(
    role_str: Option<&str>,
    blocks: &[Block],
) -> bool {
    if role_str != Some("user") {
        return false;
    }
    let mut saw_text = false;
    for text in codex_text_blocks(blocks) {
        saw_text = true;
        if !codex_is_internal_user_context_text(text) {
            return false;
        }
    }
    saw_text
}

pub(super) fn codex_is_internal_user_context_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    CODEX_INTERNAL_USER_CONTEXT_TAGS
        .iter()
        .any(|(start_tag, end_tag)| {
            trimmed.starts_with(start_tag) && text.trim_end().ends_with(end_tag)
        })
        || codex_is_agents_instructions_text(trimmed)
}

pub(super) fn codex_is_agents_instructions_text(text: &str) -> bool {
    text.starts_with("# AGENTS.md instructions") && text.contains("<INSTRUCTIONS>")
}

pub(super) fn codex_event_msg_event(
    payload: &Value,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    _raw_line: Value,
) -> Event {
    let event_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("event_msg");
    let role = match event_type {
        "user_message" => Role::User,
        "agent_message" => Role::Assistant,
        _ => Role::System,
    };

    let mut blocks = Vec::new();
    let message_text = payload.get("message").and_then(|v| v.as_str());
    let last_agent_text = payload.get("last_agent_message").and_then(|v| v.as_str());

    if let Some(text) = message_text {
        blocks.push(Block::Text {
            text: text.to_string(),
        });
    }
    if let Some(text) = last_agent_text {
        if message_text != Some(text) && !text.trim().is_empty() {
            blocks.push(Block::Text {
                text: text.to_string(),
            });
        }
    }
    blocks.push(Block::Other {
        raw: payload.clone(),
    });

    let mut event = provider_payload_event(
        format!("codex:event_msg:{}:{}", event_type, line_no),
        EventKind::Lifecycle,
        role,
        timestamp,
        event_type,
        ProviderPayloadData {
            payload: payload.clone(),
        },
    );
    event.blocks = blocks;
    event
}

pub(super) fn codex_image_block(block: &Value) -> Option<Block> {
    let mime_type = block
        .get("mime_type")
        .or_else(|| block.get("mimeType"))
        .and_then(|v| v.as_str())
        .unwrap_or("image/*")
        .to_string();
    let image_url = block
        .get("image_url")
        .or_else(|| block.get("url"))
        .or_else(|| block.get("source"))
        .and_then(|v| v.as_str())?;
    if let Some((mime, data)) = parse_data_uri(image_url) {
        return Some(Block::Image {
            mime_type: mime.to_string(),
            data: Some(data.to_string()),
            path: None,
        });
    }
    Some(Block::Image {
        mime_type,
        data: None,
        path: Some(image_url.to_string()),
    })
}

pub(super) fn parse_data_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

pub(super) struct ProviderPayloadData {
    payload: Value,
}

pub(super) fn provider_payload_event(
    id: String,
    kind: EventKind,
    role: Role,
    timestamp: chrono::DateTime<Utc>,
    _payload_kind: &str,
    data: ProviderPayloadData,
) -> Event {
    let ProviderPayloadData { payload } = data;
    Event {
        id,
        kind,
        role,
        timestamp,
        links: Links::default(),
        blocks: vec![Block::Other {
            raw: payload.clone(),
        }],
        metadata: Metadata {
            model: None,
            usage: None,
        },
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct CodexSqliteThreadMetadata {
    pub(super) cwd: Option<String>,
    pub(super) title: Option<String>,
    pub(super) rollout_path: Option<String>,
}

pub(super) fn build_sqlite_thread_metadata_lookup(
    codex_dir: &Path,
) -> Result<HashMap<String, CodexSqliteThreadMetadata>> {
    let sqlite_path = codex_dir.join(CODEX_SQLITE_FILE_BASENAME);
    if !sqlite_path.exists() {
        return Ok(HashMap::new());
    }
    let conn = rusqlite::Connection::open(&sqlite_path)?;
    if !has_table(&conn, "threads")? {
        return Ok(HashMap::new());
    }

    let has_cwd = has_columns(&conn, "threads", &["cwd"])?;
    let has_title = has_columns(&conn, "threads", &["title"])?;
    let has_rollout_path = has_columns(&conn, "threads", &["rollout_path"])?;
    if !has_cwd && !has_title && !has_rollout_path {
        return Ok(HashMap::new());
    }

    let mut map = HashMap::new();
    let query = match (has_cwd, has_title, has_rollout_path) {
        (true, true, true) => "SELECT id, cwd, title, rollout_path FROM threads",
        (true, true, false) => "SELECT id, cwd, title, NULL AS rollout_path FROM threads",
        (true, false, true) => "SELECT id, cwd, NULL AS title, rollout_path FROM threads",
        (true, false, false) => "SELECT id, cwd, NULL AS title, NULL AS rollout_path FROM threads",
        (false, true, true) => "SELECT id, NULL AS cwd, title, rollout_path FROM threads",
        (false, true, false) => "SELECT id, NULL AS cwd, title, NULL AS rollout_path FROM threads",
        (false, false, true) => "SELECT id, NULL AS cwd, NULL AS title, rollout_path FROM threads",
        (false, false, false) => unreachable!(),
    };
    let mut stmt = conn.prepare(query)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            CodexSqliteThreadMetadata {
                cwd: row.get::<_, Option<String>>(1)?,
                title: row.get::<_, Option<String>>(2)?,
                rollout_path: row.get::<_, Option<String>>(3)?,
            },
        ))
    })?;
    for (id, metadata) in rows.flatten() {
        map.insert(id, metadata);
    }
    Ok(map)
}

pub(super) fn clean_non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(super) fn select_codex_display_title(
    index_title: Option<&str>,
    sqlite_title: Option<&str>,
    rollout_title: Option<&str>,
    session_id: &str,
) -> Option<String> {
    clean_non_empty(index_title)
        .filter(|title| *title != session_id)
        .or_else(|| clean_non_empty(sqlite_title).filter(|title| *title != session_id))
        .or_else(|| clean_non_empty(rollout_title).filter(|title| *title != session_id))
        .map(str::to_string)
}

pub(super) fn resolve_codex_projection_title(
    source_path: &Path,
    session_id: &str,
    rollout_title: Option<&str>,
) -> Result<Option<String>> {
    let codex_dir = source_path
        .ancestors()
        .find(|ancestor| ancestor.file_name().and_then(|name| name.to_str()) == Some("sessions"))
        .and_then(Path::parent);
    let Some(codex_dir) = codex_dir else {
        return Ok(select_codex_display_title(
            None,
            None,
            rollout_title,
            session_id,
        ));
    };

    let index_entries = load_session_index_entries(&codex_dir.join("session_index.jsonl"))?;
    let sqlite_metadata = build_sqlite_thread_metadata_lookup(codex_dir)?;
    Ok(select_codex_display_title(
        index_entries.get(session_id).map(String::as_str),
        sqlite_metadata
            .get(session_id)
            .and_then(|metadata| metadata.title.as_deref()),
        rollout_title,
        session_id,
    ))
}

pub(super) fn resolve_codex_reindex_title(
    session: &CodexRolloutSummary,
    sqlite_metadata: Option<&CodexSqliteThreadMetadata>,
    session_states: &session_state::SessionStateStore,
) -> String {
    session_state::resolve_session_state(
        session_states,
        PROVIDER_ID,
        &session.session_id,
        session.workspace_dir.as_deref(),
    )
    .display_title
    .as_deref()
    .and_then(|title| clean_non_empty(Some(title)))
    .filter(|title| *title != session.session_id)
    .or_else(|| {
        sqlite_metadata
            .and_then(|metadata| clean_non_empty(metadata.title.as_deref()))
            .filter(|title| *title != session.session_id)
    })
    .or_else(|| {
        clean_non_empty(session.title.as_deref()).filter(|title| *title != session.session_id)
    })
    .or_else(|| clean_non_empty(session.title.as_deref()))
    .unwrap_or(&session.session_id)
    .to_string()
}

#[derive(Debug, Clone)]
pub(super) struct CodexRolloutSummary {
    pub(super) session_id: String,
    pub(super) title: Option<String>,
    pub(super) workspace_dir: Option<String>,
    pub(super) model_provider: Option<String>,
    pub(super) original_model_provider: Option<String>,
    pub(super) created_at: Option<String>,
    pub(super) updated_at: Option<String>,
    pub(super) has_user_event: bool,
}

pub(super) fn discover_codex_rollouts(
    codex_dir: &Path,
) -> Result<Vec<(PathBuf, CodexRolloutSummary)>> {
    let mut rollouts = Vec::new();
    for root in [
        codex_dir.join("sessions"),
        codex_dir.join("archived_sessions"),
    ] {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root)
            .max_depth(5)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(summary) = read_codex_rollout_summary(path)? {
                rollouts.push((path.to_path_buf(), summary));
            }
        }
    }
    Ok(rollouts)
}

pub(super) fn read_codex_rollout_summary(path: &Path) -> Result<Option<CodexRolloutSummary>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex rollout file: {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut session_id = None;
    let mut title = None;
    let mut workspace_dir = None;
    let mut model_provider = None;
    let mut created_at = None;
    let mut updated_at = None;
    let mut has_user_event = false;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if !has_user_event && rollout_value_has_user_event(&value) {
            has_user_event = true;
        }

        if let Some(timestamp) = value.get("timestamp").and_then(|value| value.as_str()) {
            created_at.get_or_insert_with(|| timestamp.to_string());
            updated_at = Some(timestamp.to_string());
        }

        if value.get("type").and_then(|value| value.as_str()) != Some("session_meta") {
            continue;
        }

        let Some(payload) = value.get("payload") else {
            continue;
        };
        session_id = payload
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(session_id);
        title = payload
            .get("title")
            .or_else(|| payload.get("thread_name"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(title);
        workspace_dir = payload
            .get("cwd")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(workspace_dir);
        model_provider = payload
            .get("model_provider")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(model_provider);
    }

    let Some(session_id) = session_id else {
        return Ok(None);
    };

    Ok(Some(CodexRolloutSummary {
        session_id,
        title,
        workspace_dir,
        original_model_provider: model_provider.clone(),
        model_provider,
        created_at,
        updated_at,
        has_user_event,
    }))
}

pub(super) fn rollout_value_has_user_event(value: &Value) -> bool {
    if value.get("type").and_then(|value| value.as_str()) == Some("event_msg")
        && value
            .get("payload")
            .and_then(|payload| payload.get("type"))
            .and_then(|value| value.as_str())
            == Some("user_message")
    {
        return true;
    }

    let Some(payload) = value.get("payload") else {
        return false;
    };
    if payload.get("type").and_then(|value| value.as_str()) == Some("message")
        && payload.get("role").and_then(|value| value.as_str()) == Some("user")
    {
        return true;
    }

    false
}

pub(super) fn load_session_index_entries(index_path: &Path) -> Result<HashMap<String, String>> {
    if !index_path.exists() {
        return Ok(HashMap::new());
    }

    let file = File::open(index_path).with_context(|| {
        format!(
            "Failed to open Codex session index: {}",
            index_path.display()
        )
    })?;
    let reader = BufReader::new(file);
    let mut entries = HashMap::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let id = value.get("id").and_then(|value| value.as_str());
        let thread_name = value
            .get("thread_name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if let Some(id) = id {
            entries.insert(id.to_string(), thread_name.to_string());
        }
    }
    Ok(entries)
}

pub(super) fn append_session_index_entry(
    index_path: &Path,
    session_id: &str,
    title: &str,
    updated_at: Option<&str>,
) -> Result<()> {
    if let Some(parent) = index_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut index_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(index_path)?;
    let updated_at = updated_at
        .map(str::to_string)
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    writeln!(
        index_file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "id": session_id,
            "thread_name": title,
            "updated_at": updated_at,
        }))?
    )?;
    Ok(())
}

pub(super) fn update_session_index_entry(
    index_path: &Path,
    session_id: &str,
    new_title: &str,
) -> Result<()> {
    if !index_path.exists() {
        anyhow::bail!("Codex session index not found");
    }

    let content = std::fs::read_to_string(index_path)?;
    let mut new_lines = Vec::new();
    let mut found = false;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => {
                new_lines.push(line.to_string());
                continue;
            }
        };
        if value.get("id").and_then(|value| value.as_str()) == Some(session_id) {
            if let Value::Object(ref mut map) = value {
                map.insert(
                    "thread_name".to_string(),
                    Value::String(new_title.to_string()),
                );
                found = true;
            }
            new_lines.push(serde_json::to_string(&value)?);
        } else {
            new_lines.push(line.to_string());
        }
    }

    if !found {
        anyhow::bail!("Codex session not found in index: {}", session_id);
    }

    std::fs::write(index_path, new_lines.join("\n") + "\n")?;
    Ok(())
}

pub(super) fn rewrite_rollout_model_provider(path: &Path, model_provider: &str) -> Result<()> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex rollout file: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut updated = false;

    for line in reader.lines() {
        let line = line?;
        if updated || line.trim().is_empty() {
            lines.push(line);
            continue;
        }
        let mut value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                lines.push(line);
                continue;
            }
        };
        if value.get("type").and_then(|value| value.as_str()) == Some("session_meta") {
            if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
                payload.insert(
                    "model_provider".to_string(),
                    Value::String(model_provider.to_string()),
                );
                updated = true;
                lines.push(serde_json::to_string(&value)?);
                continue;
            }
        }
        lines.push(line);
    }

    if !updated {
        anyhow::bail!(
            "Codex rollout file is missing session_meta payload: {}",
            path.display()
        );
    }

    std::fs::write(path, lines.join("\n") + "\n")
        .with_context(|| format!("Failed to write Codex rollout file: {}", path.display()))?;
    Ok(())
}

pub(super) fn extract_cwd_from_session_file(id: &str) -> Option<String> {
    let path = find_session_file(id)?;
    extract_cwd_from_session_path(&path)
}

pub(super) fn extract_cwd_from_session_path(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(5) {
        let line = line.ok()?;
        let value: Value = serde_json::from_str(&line).ok()?;
        if value.get("type").and_then(|v| v.as_str()) == Some("session_meta") {
            return value
                .get("payload")
                .and_then(|p| p.get("cwd"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
    }
    None
}

pub(super) fn find_session_file(id: &str) -> Option<PathBuf> {
    // Search both active sessions and archived sessions
    let dirs = [
        get_codex_dir().join("sessions"),
        get_codex_dir().join("archived_sessions"),
    ];

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(dir)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains(id))
                .unwrap_or(false)
            {
                return Some(path.to_path_buf());
            }
        }
    }

    None
}

pub(super) fn build_session_file_lookup(
    codex_dir: &Path,
    session_ids: &[String],
) -> HashMap<String, CodexSessionFileMeta> {
    let mut lookup = HashMap::new();
    if session_ids.is_empty() {
        return lookup;
    }

    let mut remaining: HashSet<String> = session_ids.iter().cloned().collect();
    let dirs = [
        codex_dir.join("sessions"),
        codex_dir.join("archived_sessions"),
    ];

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(dir)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if remaining.is_empty() {
                return lookup;
            }

            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(session_id) = remaining
                .iter()
                .find(|id| file_name.contains(id.as_str()))
                .cloned()
            else {
                continue;
            };
            let size_bytes = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            lookup.insert(session_id.clone(), CodexSessionFileMeta { size_bytes });
            remaining.remove(&session_id);
        }
    }

    lookup
}
