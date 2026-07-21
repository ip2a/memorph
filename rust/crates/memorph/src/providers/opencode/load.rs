use super::*;

pub(super) fn opencode_session_id_from_source_locator(source_locator: &str) -> Result<String> {
    if let Some((_, session_id)) = opencode_database_source(source_locator) {
        return Ok(session_id.to_string());
    }
    anyhow::ensure!(
        !source_locator.contains('#'),
        "OpenCode database source locator is invalid"
    );

    let source_path = Path::new(source_locator);
    if source_path.is_file() {
        let source: Value =
            serde_json::from_reader(File::open(source_path)?).with_context(|| {
                format!(
                    "Failed to parse OpenCode session source: {}",
                    source_path.display()
                )
            })?;
        return source
            .get("id")
            .and_then(Value::as_str)
            .filter(|session_id| !session_id.is_empty())
            .map(str::to_string)
            .context("OpenCode session source has no id");
    }

    if !source_locator.is_empty() && !source_locator.contains('/') && !source_locator.contains('\\')
    {
        return Ok(source_locator.to_string());
    }

    anyhow::bail!("OpenCode source locator does not exist: {source_locator}")
}

pub(super) fn import_canonical_session_from_source(
    session_id: &str,
    source_locator: &str,
) -> Result<ImportedSession> {
    let data = if let Some((database_path, locator_session_id)) =
        opencode_database_source(source_locator)
    {
        anyhow::ensure!(
            locator_session_id == session_id,
            "OpenCode database source locator session does not match projected session"
        );
        load_session_from_db_path(Path::new(database_path), session_id)?
    } else {
        let source_path = Path::new(source_locator);
        if source_path.is_file() {
            load_session_from_filesystem_path(session_id, source_path)?
        } else {
            anyhow::bail!(
                "OpenCode source locator does not identify a native source plane: {source_locator}"
            );
        }
    };
    imported_session_from_data(session_id, data)
}

/// Load a single page of an OpenCode session by event (message) range.
///
/// OpenCode stores sessions as structured rows/files rather than an append-only
/// line stream, so pagination is native to the source plane: the database plane
/// uses SQL LIMIT/OFFSET and the filesystem plane skips/takes sorted message
/// files. No separate byte-offset index is needed because the source itself is
/// the index. `event_count` is the total message count for the session.
///
/// `message_count` (visible messages across the whole session) and the page
/// events are derived from the same canonical mapping used by a full import, so
/// counts and per-page visibility stay identical. The full message list is read
/// for counting (cheap: SQL rows / file list) but only the requested page is
/// materialized into canonical events, which is where the prior full-import
/// cost came from.
pub(super) fn import_opencode_session_page(
    source_locator: &str,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<ProviderSessionImportPage> {
    let session_id = opencode_session_id_from_source_locator(source_locator)?;

    let (session_json, messages_page, parts_page, total_message_count, full_message_count) =
        if let Some((database_path, locator_session_id)) = opencode_database_source(source_locator)
        {
            anyhow::ensure!(
                locator_session_id == session_id,
                "OpenCode database source locator session does not match projected session"
            );
            load_session_page_from_db_path(
                Path::new(database_path),
                &session_id,
                event_offset,
                event_limit,
            )?
        } else {
            let source_path = Path::new(source_locator);
            if source_path.is_file() {
                load_session_page_from_filesystem_path(
                    &session_id,
                    source_path,
                    event_offset,
                    event_limit,
                )?
            } else {
                anyhow::bail!(
                    "OpenCode source locator does not identify a native source plane: {source_locator}"
                );
            }
        };

    let imported =
        imported_session_from_data(&session_id, (session_json, messages_page, parts_page))?;

    let turns = project_session_turns(
        &imported.session.identity.canonical_id,
        &imported.session.events,
        TurnQuality::Inferred,
    );
    let turn_count = (event_offset == 0 && imported.session.events.len() == total_message_count)
        .then_some(turns.len());
    Ok(ProviderSessionImportPage {
        imported,
        event_count: total_message_count,
        message_count: full_message_count,
        turn_count,
        turns,
    })
}

pub(super) fn opencode_database_source(source_locator: &str) -> Option<(&str, &str)> {
    let (database_path, session_id) = source_locator.rsplit_once("#session=")?;
    (!database_path.is_empty() && !session_id.is_empty()).then_some((database_path, session_id))
}

pub(super) fn imported_session_from_data(
    session_id: &str,
    (session_json, messages, parts): (
        Value,
        Vec<(Option<i64>, Value)>,
        HashMap<String, Vec<Value>>,
    ),
) -> Result<ImportedSession> {
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::new();
    let mut artifacts = Vec::new();

    let mut msg_list: Vec<(Option<i64>, Value, Vec<Value>)> = messages
        .into_iter()
        .map(|(created, msg_json)| {
            let msg_id = msg_json
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let msg_parts: Vec<Value> = parts.get(&msg_id).cloned().unwrap_or_default();
            (created, msg_json, msg_parts)
        })
        .collect();
    msg_list.sort_by(|(left_created, left, _), (right_created, right, _)| {
        let left_id = left.get("id").and_then(Value::as_str).unwrap_or("");
        let right_id = right.get("id").and_then(Value::as_str).unwrap_or("");
        left_created
            .cmp(right_created)
            .then_with(|| left_id.cmp(right_id))
    });

    for (source_order, (source_created, msg_json, msg_parts)) in msg_list.into_iter().enumerate() {
        let role_str = msg_json
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("user");
        let role = match role_str {
            "user" => EventRole::User,
            "assistant" => EventRole::Assistant,
            other => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: MappingDisposition::Normalized,
                    code: "unknown_role_normalized".to_string(),
                    message: format!("Normalized unknown OpenCode role '{}'", other),
                    path: None,
                    raw: Some(msg_json.clone()),
                });
                EventRole::Unknown
            }
        };

        let msg_id = msg_json
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let parent_id = msg_json
            .get("parentID")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let timestamp = msg_json
            .get("time")
            .and_then(|v| v.get("created"))
            .and_then(|v| v.as_i64())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .or_else(|| source_created.and_then(chrono::DateTime::from_timestamp_millis))
            .unwrap_or_else(|| {
                chrono::DateTime::from_timestamp_millis(source_order as i64)
                    .expect("OpenCode message source order is a valid timestamp")
            });

        let mut blocks =
            canonical_blocks_from_parts(&msg_id, &msg_parts, &mut report, &mut artifacts);
        if blocks.is_empty() {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: MappingDisposition::Normalized,
                code: "opencode_message_without_mappable_parts".to_string(),
                message:
                    "OpenCode message had no mappable parts and was preserved as provider payload"
                        .to_string(),
                path: Some(format!("message:{}", msg_id)),
                raw: Some(msg_json.clone()),
            });
            blocks = vec![EventBlock::ProviderPayload {
                kind: "message_without_mappable_parts".to_string(),
                payload: msg_json.clone(),
            }];
        }

        let model = msg_json
            .get("modelID")
            .or_else(|| msg_json.get("model").and_then(|m| m.get("modelID")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let provider = msg_json
            .get("providerID")
            .or_else(|| msg_json.get("model").and_then(|m| m.get("providerID")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| PROVIDER_ID.to_string());

        let usage = msg_json.get("tokens").map(|t| UsageStats {
            input_tokens: t.get("input").and_then(|v| v.as_u64()),
            output_tokens: t.get("output").and_then(|v| v.as_u64()),
            total_tokens: t.get("total").and_then(|v| v.as_u64()),
        });

        let finish = msg_json
            .get("finish")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let turn_boundary = opencode_turn_boundary(finish.as_deref());
        let cost = msg_json.get("cost").and_then(|v| v.as_f64());
        let agent = msg_json
            .get("agent")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let mode = msg_json
            .get("mode")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut provider_ext = BTreeMap::new();
        provider_ext.insert("opencode_message".to_string(), msg_json.clone());
        if let Some(finish) = finish {
            provider_ext.insert("finish".to_string(), Value::String(finish));
        }
        if let Some(cost) = cost {
            provider_ext.insert("cost".to_string(), Value::from(cost));
        }
        if let Some(agent) = agent {
            provider_ext.insert("agent".to_string(), Value::String(agent));
        }
        if let Some(mode) = mode {
            provider_ext.insert("mode".to_string(), Value::String(mode));
        }

        let kind = derive_event_kind(&blocks);
        events.push(SessionEvent {
            id: msg_id.clone(),
            kind,
            role,
            timestamp,
            links: EventLinks {
                parent_event_id: parent_id.clone(),
                provider_parent_id: parent_id,
                provider_turn_id: None,
                turn_index: None,
                turn_boundary,
                related_event_ids: Vec::new(),
            },
            blocks,
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: provider,
                    original_id: Some(msg_id),
                    original_role: Some(role_str.to_string()),
                    phase: None,
                },
                model,
                usage,
                fidelity: MappingDisposition::Preserved,
                provider_ext,
            },
        });
    }

    let session_id_val = session_json
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or(session_id)
        .to_string();
    let title = session_json
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let project_dir = session_json
        .get("directory")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let created = session_json
        .get("time")
        .and_then(|v| v.get("created"))
        .and_then(|v| v.as_i64())
        .and_then(chrono::DateTime::from_timestamp_millis);
    let updated = session_json
        .get("time")
        .and_then(|v| v.get("updated"))
        .and_then(|v| v.as_i64())
        .and_then(chrono::DateTime::from_timestamp_millis);

    let mut extensions = BTreeMap::new();
    extensions.insert("opencode_session".to_string(), session_json.clone());

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: session_id_val.clone(),
                source_title: title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: session_id_val.clone(),
                    source_path: Some(session_id.to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: project_dir,
                created_at: created,
                last_active_at: updated,
                tags: Vec::new(),
            },
            events,
            artifacts,
            extensions,
        },
        report,
    })
}

pub(super) fn opencode_turn_boundary(finish: Option<&str>) -> Option<TurnBoundary> {
    match finish {
        Some("stop") => Some(TurnBoundary::Completed),
        Some("error") => Some(TurnBoundary::Failed),
        Some("abort" | "cancelled" | "canceled" | "length" | "content_filter") => {
            Some(TurnBoundary::Interrupted)
        }
        _ => None,
    }
}

pub(super) fn canonical_blocks_from_parts(
    msg_id: &str,
    msg_parts: &[Value],
    report: &mut MappingReport,
    artifacts: &mut Vec<SessionArtifact>,
) -> Vec<EventBlock> {
    let mut blocks = Vec::new();

    for (idx, part) in msg_parts.iter().enumerate() {
        let part_type = part.get("type").and_then(|v| v.as_str());
        match part_type {
            Some("text") => {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    blocks.push(EventBlock::Text {
                        text: text.to_string(),
                    });
                } else {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: MappingDisposition::Normalized,
                        code: "opencode_text_part_missing_text".to_string(),
                        message:
                            "OpenCode text part without text was preserved as provider payload"
                                .to_string(),
                        path: Some(format!("{}:part:{}", msg_id, idx)),
                        raw: Some(part.clone()),
                    });
                    blocks.push(EventBlock::ProviderPayload {
                        kind: "text".to_string(),
                        payload: part.clone(),
                    });
                }
            }
            Some("reasoning") => {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    blocks.push(EventBlock::Thinking {
                        text: text.to_string(),
                        signature: None,
                    });
                } else {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: MappingDisposition::Normalized,
                        code: "opencode_reasoning_part_missing_text".to_string(),
                        message:
                            "OpenCode reasoning part without text was preserved as provider payload"
                                .to_string(),
                        path: Some(format!("{}:part:{}", msg_id, idx)),
                        raw: Some(part.clone()),
                    });
                    blocks.push(EventBlock::ProviderPayload {
                        kind: "reasoning".to_string(),
                        payload: part.clone(),
                    });
                }
            }
            Some("tool") => {
                let call_id = part
                    .get("callID")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool_name = part
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let state = part.get("state").cloned().unwrap_or_default();
                let input = state.get("input").cloned();
                let output = state
                    .get("output")
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string())
                    })
                    .unwrap_or_default();
                let status = state
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("completed");

                if tool_name != "unknown" || input.is_some() {
                    blocks.push(EventBlock::ToolCall {
                        tool_call_id: call_id.clone(),
                        name: tool_name,
                        input,
                    });
                }
                blocks.push(EventBlock::ToolResult {
                    tool_call_id: call_id,
                    content: output,
                    is_error: status == "error",
                });
            }
            Some("file") => {
                let mime = part
                    .get("mime")
                    .and_then(|v| v.as_str())
                    .unwrap_or("application/octet-stream");
                let filename = part
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("file");
                let url = part.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if mime.starts_with("image/") && url.starts_with("data:") {
                    if let Some((mime_type, data)) = parse_data_uri(url) {
                        blocks.push(EventBlock::Image {
                            mime_type: mime_type.to_string(),
                            data: Some(data.to_string()),
                            path: Some(filename.to_string()),
                        });
                        artifacts.push(SessionArtifact {
                            id: format!("{}:image:{}", msg_id, idx),
                            kind: ArtifactKind::Image,
                            path: Some(filename.to_string()),
                            mime_type: Some(mime_type.to_string()),
                            content: None,
                            metadata: BTreeMap::new(),
                        });
                    } else {
                        report.push_issue(MappingIssue {
                            level: MappingIssueLevel::Warning,
                            disposition: MappingDisposition::Normalized,
                            code: "opencode_image_part_invalid_data_uri".to_string(),
                            message: "OpenCode image part with an invalid data URI was preserved as provider payload"
                                .to_string(),
                            path: Some(format!("{}:part:{}", msg_id, idx)),
                            raw: Some(part.clone()),
                        });
                        blocks.push(EventBlock::ProviderPayload {
                            kind: "file".to_string(),
                            payload: part.clone(),
                        });
                    }
                } else if !url.is_empty() {
                    blocks.push(EventBlock::File {
                        path: filename.to_string(),
                        content: Some(url.to_string()),
                        mime_type: Some(mime.to_string()),
                    });
                    artifacts.push(SessionArtifact {
                        id: format!("{}:file:{}", msg_id, idx),
                        kind: ArtifactKind::File,
                        path: Some(filename.to_string()),
                        mime_type: Some(mime.to_string()),
                        content: Some(url.to_string()),
                        metadata: BTreeMap::new(),
                    });
                } else {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: MappingDisposition::Normalized,
                        code: "opencode_file_part_missing_url".to_string(),
                        message:
                            "OpenCode file part without a URL was preserved as provider payload"
                                .to_string(),
                        path: Some(format!("{}:part:{}", msg_id, idx)),
                        raw: Some(part.clone()),
                    });
                    blocks.push(EventBlock::ProviderPayload {
                        kind: "file".to_string(),
                        payload: part.clone(),
                    });
                }
            }
            Some("patch") => {
                let files = part
                    .get("files")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let hash = part
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let diff_text = part
                    .get("text")
                    .or_else(|| part.get("diff"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                blocks.push(EventBlock::Patch {
                    summary: None,
                    diff_text: diff_text.clone(),
                    files: files.clone(),
                    hash: hash.clone(),
                });
                artifacts.push(SessionArtifact {
                    id: format!("{}:patch:{}", msg_id, idx),
                    kind: ArtifactKind::Patch,
                    path: None,
                    mime_type: None,
                    content: diff_text,
                    metadata: {
                        let mut metadata = BTreeMap::new();
                        if let Some(hash) = hash {
                            metadata.insert("hash".to_string(), Value::String(hash));
                        }
                        if !files.is_empty() {
                            metadata.insert(
                                "files".to_string(),
                                Value::Array(files.into_iter().map(Value::String).collect()),
                            );
                        }
                        metadata
                    },
                });
            }
            Some("step-start") | Some("step-finish") | Some("compaction") => {
                blocks.push(EventBlock::ProviderPayload {
                    kind: part_type.unwrap_or("unknown").to_string(),
                    payload: part.clone(),
                });
            }
            Some(other) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: MappingDisposition::Normalized,
                    code: "unknown_part_preserved".to_string(),
                    message: format!("Preserved unknown OpenCode part '{}'", other),
                    path: Some(format!("{}:part:{}", msg_id, idx)),
                    raw: Some(part.clone()),
                });
                blocks.push(EventBlock::Unknown { raw: part.clone() });
            }
            None => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Normalized,
                    code: "missing_part_type".to_string(),
                    message: "OpenCode part without a type was preserved as unknown payload"
                        .to_string(),
                    path: Some(format!("{}:part:{}", msg_id, idx)),
                    raw: Some(part.clone()),
                });
                blocks.push(EventBlock::Unknown { raw: part.clone() });
            }
        }
    }

    blocks
}

pub(super) fn derive_event_kind(blocks: &[EventBlock]) -> SessionEventKind {
    if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::Patch { .. }))
    {
        SessionEventKind::Patch
    } else if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolResult { .. }))
    {
        SessionEventKind::ToolResult
    } else if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolCall { .. }))
    {
        SessionEventKind::ToolCall
    } else if blocks.iter().any(|block| {
        matches!(
            block,
            EventBlock::ProviderPayload { .. } | EventBlock::Unknown { .. }
        )
    }) {
        SessionEventKind::Unknown
    } else {
        SessionEventKind::Message
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
