use super::*;

pub(super) fn export_canonical_session(
    session: &CanonicalSession,
    target_dir: &Path,
) -> Result<String> {
    export_canonical_session_in_codex_dir(session, target_dir, &get_codex_dir())
}

pub(super) fn export_canonical_session_in_codex_dir(
    session: &CanonicalSession,
    target_dir: &Path,
    codex_dir: &Path,
) -> Result<String> {
    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let timestamp_str = now.format("%Y-%m-%dT%H-%M-%S").to_string();
    let filename = format!("rollout-{}-{}.jsonl", timestamp_str, session_id);
    let sessions_dir = codex_dir
        .join("sessions")
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string());
    std::fs::create_dir_all(&sessions_dir)?;
    let file_path = sessions_dir.join(filename);
    write_canonical_codex_rollout(
        session,
        target_dir,
        codex_dir,
        &session_id,
        &file_path,
        now,
        true,
    )?;
    Ok(session_id)
}

pub(super) fn write_canonical_codex_rollout(
    session: &CanonicalSession,
    target_dir: &Path,
    codex_dir: &Path,
    session_id: &str,
    file_path: &Path,
    now: chrono::DateTime<Utc>,
    update_registry: bool,
) -> Result<()> {
    let rollout_path = file_path.to_string_lossy().to_string();
    let mut file = File::create(file_path)?;
    let git_info = get_git_info(target_dir);
    let codex_version = get_codex_version_in_codex_dir(codex_dir);
    let codex_model_provider = read_codex_model_provider(codex_dir);
    let target_dir_str = target_dir.to_string_lossy().to_string();
    let title = canonical_session_title(session);
    let first_user_message = first_user_message(session);
    let has_user_event = has_user_event(session);
    let base_instructions = canonical_session_instruction_context_text(session);

    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": now.to_rfc3339(),
            "type": "session_meta",
            "payload": {
                "id": session_id,
                "timestamp": now.to_rfc3339(),
                "cwd": target_dir_str,
                "originator": "memorph-cli",
                "cli_version": codex_version,
                "source": "cli",
                "model_provider": codex_model_provider,
                "title": title,
                "base_instructions": base_instructions.as_ref().map(|text| {
                    serde_json::json!({ "text": text })
                }).unwrap_or(Value::Null),
                "git": {
                    "commit_hash": git_info.as_ref().and_then(|git| git.commit_hash.clone()).unwrap_or_default(),
                    "branch": git_info.as_ref().and_then(|git| git.branch.clone()).unwrap_or_default(),
                }
            }
        }))?
    )?;

    let turn_id = Uuid::new_v4().to_string();
    let first_ts = session
        .events
        .first()
        .map(|event| event.timestamp)
        .unwrap_or(now);
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": first_ts.to_rfc3339(),
            "type": "event_msg",
            "payload": {
                "type": "task_started",
                "turn_id": turn_id,
                "started_at": first_ts.timestamp(),
                "collaboration_mode_kind": "default"
            }
        }))?
    )?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": first_ts.to_rfc3339(),
            "type": "turn_context",
            "payload": {
                "turn_id": turn_id,
                "cwd": target_dir_str,
                "current_date": first_ts.format("%Y-%m-%d").to_string(),
                "timezone": "Asia/Shanghai"
            }
        }))?
    )?;

    let mut wrote_user_event = false;
    let mut last_agent_message = String::new();
    for event in &session.events {
        if let Some(segment) = compression::compressed_segment(event) {
            write_codex_compacted_rollout_item(&mut file, event, segment)?;
            if event.role == EventRole::Assistant {
                last_agent_message = segment.summary.to_string();
            }
            continue;
        }
        let Some(visible_role) = canonical_event_visible_message_role(event) else {
            continue;
        };
        let role = match visible_role {
            EventRole::Assistant => "assistant",
            EventRole::User | EventRole::Tool => "user",
            EventRole::System | EventRole::Developer | EventRole::Unknown => continue,
        };
        let content = canonical_event_to_codex_content(event);
        if content.is_empty() {
            continue;
        }
        let mut payload = serde_json::json!({
            "type": "message",
            "role": role,
            "content": content,
        });
        if event.role == EventRole::Assistant {
            payload["phase"] = Value::String("final_answer".to_string());
            last_agent_message = canonical_event_visible_text(event);
            writeln!(
                file,
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "timestamp": event.timestamp.to_rfc3339(),
                    "type": "event_msg",
                    "payload": {
                        "type": "agent_message",
                        "message": last_agent_message,
                        "phase": "final_answer",
                        "memory_citation": null
                    }
                }))?
            )?;
        }
        writeln!(
            file,
            "{}",
            serde_json::to_string(&serde_json::json!({
                "timestamp": event.timestamp.to_rfc3339(),
                "type": "response_item",
                "payload": payload,
            }))?
        )?;
        if visible_role == EventRole::User && !wrote_user_event {
            let user_text = canonical_event_visible_text(event);
            writeln!(
                file,
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "timestamp": event.timestamp.to_rfc3339(),
                    "type": "event_msg",
                    "payload": {
                        "type": "user_message",
                        "message": user_text,
                        "images": [],
                        "local_images": [],
                        "text_elements": []
                    }
                }))?
            )?;
            wrote_user_event = true;
        }
    }

    let last_ts = session
        .events
        .last()
        .map(|event| event.timestamp)
        .unwrap_or(now);
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": last_ts.to_rfc3339(),
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "turn_id": turn_id,
                "last_agent_message": last_agent_message,
                "completed_at": last_ts.timestamp(),
                "duration_ms": 1000
            }
        }))?
    )?;

    file.flush()?;
    file.sync_all()?;
    if update_registry {
        let index_path = codex_dir.join("session_index.jsonl");
        let mut index_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&index_path)?;
        writeln!(
            index_file,
            "{}",
            serde_json::to_string(&serde_json::json!({
                "id": session_id,
                "thread_name": title,
                "updated_at": now.to_rfc3339(),
            }))?
        )?;
        update_codex_sqlite(CodexSqliteUpdate {
            codex_dir,
            session_id,
            rollout_path: &rollout_path,
            cwd: target_dir,
            title: &title,
            first_user_message: first_user_message.as_deref(),
            has_user_event,
            now: &now,
        })?;
        update_codex_global_state_file_if_exists(codex_dir, target_dir)?;
    }
    Ok(())
}

pub(super) fn replace_codex_session(session_id: &str, session: &CanonicalSession) -> Result<()> {
    let codex_dir = get_codex_dir();
    let rollout_path = find_session_file(session_id)
        .with_context(|| format!("Codex session not found: {session_id}"))?;
    let target_dir = extract_cwd_from_session_path(&rollout_path)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let temp_path = rollout_path.with_extension(format!("jsonl.memorph-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        write_canonical_codex_rollout(
            session,
            &target_dir,
            &codex_dir,
            session_id,
            &temp_path,
            Utc::now(),
            false,
        )?;
        let imported = import_canonical_session(&temp_path)?;
        if imported.session.identity.canonical_id != session_id {
            anyhow::bail!("Codex replacement validation changed session identity");
        }
        std::fs::rename(&temp_path, &rollout_path).with_context(|| {
            format!(
                "Failed to atomically replace Codex rollout: {}",
                rollout_path.display()
            )
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

fn write_codex_compacted_rollout_item(
    file: &mut impl Write,
    event: &SessionEvent,
    segment: CompressedSegment<'_>,
) -> Result<()> {
    let model_visible_summary = codex_compacted_history_text(segment);
    let source_event_count = segment.source_event_count.or_else(|| {
        (!segment.source_event_ids.is_empty()).then_some(segment.source_event_ids.len())
    });
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": event.timestamp.to_rfc3339(),
            "type": "compacted",
            "payload": {
                "message": segment.summary,
                "replacement_history": [
                    {
                        "type": "message",
                        "role": "user",
                        "content": [
                            {
                                "type": "input_text",
                                "text": model_visible_summary,
                            }
                        ]
                    }
                ],
                "memorph": {
                    "source_provider_id": segment.source_provider_id,
                    "summary": segment.summary,
                    "source_event_ids": segment.source_event_ids,
                    "source_event_count": source_event_count,
                    "archive_ref": segment.archive_ref,
                }
            }
        }))?
    )?;
    Ok(())
}

fn codex_compacted_history_text(segment: CompressedSegment<'_>) -> String {
    let mut parts = vec![
        format!(
            "[Compressed session segment from {}]",
            segment.source_provider_id
        ),
        segment.summary.to_string(),
    ];
    let source_event_count = segment
        .source_event_count
        .unwrap_or(segment.source_event_ids.len());
    if source_event_count > 0 {
        parts.push(format!("Source event count: {}", source_event_count));
    }
    if let Some(archive_ref) = segment.archive_ref {
        parts.push(format!("Archive: {}", archive_ref));
        parts.push(compression_retrieval_hint(archive_ref));
    }
    parts.join("\n")
}

pub(super) fn canonical_event_to_codex_content(event: &SessionEvent) -> Vec<Value> {
    event
        .blocks
        .iter()
        .filter_map(|block| match block {
            EventBlock::Text { text } => Some(serde_json::json!({
                "type": if event.role == EventRole::Assistant { "output_text" } else { "input_text" },
                "text": text,
            })),
            EventBlock::Thinking { text, .. } => Some(serde_json::json!({
                "type": "output_text",
                "text": format!("[Thinking]\n{}", text),
            })),
            EventBlock::Image { data: Some(data), .. } if event.role != EventRole::Assistant => {
                Some(serde_json::json!({
                    "type": "input_image",
                    "image_url": data,
                }))
            }
            EventBlock::ProviderPayload { .. } => None,
            _ => {
                let text = canonical_visible_block_text(block)?;
                (!text.trim().is_empty()).then(|| serde_json::json!({
                    "type": if event.role == EventRole::Assistant { "output_text" } else { "input_text" },
                    "text": text,
                }))
            }
        })
        .collect()
}
