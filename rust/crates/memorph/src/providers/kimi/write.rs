use super::*;

pub(super) fn write_kimi_file_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Kimi file has no parent directory")?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Kimi file name is not valid UTF-8")?;
    let temporary_path = parent.join(format!(".{file_name}.memorph-{}.tmp", Uuid::new_v4()));
    let write_result = (|| -> Result<()> {
        let mut file = File::create(&temporary_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary_path, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    write_result
}

pub(super) fn write_kimi_state_atomically(state_path: &Path, bytes: &[u8]) -> Result<()> {
    write_kimi_file_atomically(state_path, bytes)
}

pub(super) fn register_exported_kimi_session(target_dir: &Path, session_id: &str) -> Result<()> {
    let metadata_path = get_kimi_json_path();
    let mut metadata = if metadata_path.exists() {
        let raw = std::fs::read_to_string(&metadata_path)
            .with_context(|| format!("Failed to read kimi.json: {}", metadata_path.display()))?;
        serde_json::from_str::<Value>(&raw)
            .with_context(|| format!("Failed to parse kimi.json: {}", metadata_path.display()))?
    } else {
        serde_json::json!({})
    };
    let metadata_object = metadata
        .as_object_mut()
        .context("Kimi kimi.json root must be an object")?;
    let work_dirs = metadata_object
        .entry("work_dirs")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .context("Kimi kimi.json work_dirs must be an array")?;
    let target_path = target_dir.to_string_lossy().to_string();
    let existing = work_dirs.iter_mut().find(|entry| {
        entry.get("path").and_then(Value::as_str) == Some(target_path.as_str())
            && entry.get("kaos").and_then(Value::as_str).unwrap_or("local") == "local"
    });
    if let Some(entry) = existing {
        let entry_object = entry
            .as_object_mut()
            .context("Kimi kimi.json work-dir entry must be an object")?;
        entry_object.insert(
            "last_session_id".to_string(),
            Value::String(session_id.to_string()),
        );
    } else {
        work_dirs.push(serde_json::json!({
            "path": target_path,
            "kaos": "local",
            "last_session_id": session_id
        }));
    }
    let updated = serde_json::to_vec_pretty(&metadata)?;
    write_kimi_file_atomically(&metadata_path, &updated)
        .with_context(|| format!("Failed to update kimi.json: {}", metadata_path.display()))
}

#[cfg(test)]
pub(super) fn set_test_kimi_sessions_dir(path: Option<PathBuf>) {
    *TEST_KIMI_SESSIONS_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi sessions dir lock") = path;
}

#[cfg(test)]
pub(super) fn set_test_kimi_mutation_failure(mutation: Option<ProviderSourceMutation>) {
    *TEST_KIMI_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi mutation failure lock") = mutation;
}

#[cfg(test)]
pub(super) fn fail_kimi_mutation_after_write(mutation: ProviderSourceMutation) -> Result<()> {
    let mut failure = TEST_KIMI_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi mutation failure lock");
    if *failure == Some(mutation) {
        *failure = None;
        anyhow::bail!("injected Kimi mutation failure after provider write");
    }
    Ok(())
}

#[cfg(not(test))]
pub(super) fn fail_kimi_mutation_after_write(_mutation: ProviderSourceMutation) -> Result<()> {
    Ok(())
}

pub(super) fn export_canonical_session(session: &Session, target_dir: &Path) -> Result<String> {
    let session_id = Uuid::new_v4().to_string();
    let project_hash = md5_hex(target_dir.to_string_lossy().as_bytes());
    let session_dir = get_kimi_sessions_dir()
        .join(&project_hash)
        .join(&session_id);
    std::fs::create_dir_all(&session_dir)?;

    let wire_path = session_dir.join("wire.jsonl");
    let context_path = session_dir.join("context.jsonl");
    let state_path = session_dir.join("state.json");
    let mut wire_file = File::create(&wire_path)?;
    let mut context_file = File::create(&context_path)?;

    writeln!(
        wire_file,
        "{}",
        serde_json::json!({"type": "metadata", "protocol_version": "1.9"})
    )?;

    for event in &session.events {
        let Some(visible_role) = event_visible_message_role(event) else {
            continue;
        };
        let ts = event.timestamp.timestamp_millis() as f64 / 1000.0;
        match visible_role {
            Role::Assistant => {
                let content_parts = event
                    .blocks
                    .iter()
                    .filter_map(block_to_kimi_content_part)
                    .collect::<Vec<_>>();
                if content_parts.is_empty() {
                    continue;
                }
                for payload in &content_parts {
                    writeln!(
                        wire_file,
                        "{}",
                        serde_json::json!({
                            "timestamp": ts,
                            "message": {
                                "type": "ContentPart",
                                "payload": payload
                            }
                        })
                    )?;
                }
                writeln!(
                    context_file,
                    "{}",
                    serde_json::json!({
                        "role": "assistant",
                        "content": content_parts
                    })
                )?;
            }
            _ => {
                let Some(text) = event_visible_message_text(event) else {
                    continue;
                };
                writeln!(
                    wire_file,
                    "{}",
                    serde_json::json!({
                        "timestamp": ts,
                        "message": {
                            "type": "TurnBegin",
                            "payload": {
                                "user_input": [{"type": "text", "text": text}]
                            }
                        }
                    })
                )?;
                writeln!(
                    wire_file,
                    "{}",
                    serde_json::json!({
                        "timestamp": ts,
                        "message": {
                            "type": "StepBegin",
                            "payload": {"n": 1}
                        }
                    })
                )?;
                writeln!(
                    context_file,
                    "{}",
                    serde_json::json!({"role": "user", "content": text})
                )?;
            }
        }
    }

    let end_ts = session
        .context
        .last_active_at
        .or_else(|| session.events.last().map(|event| event.timestamp))
        .unwrap_or_else(Utc::now)
        .timestamp_millis() as f64
        / 1000.0;
    writeln!(
        wire_file,
        "{}",
        serde_json::json!({
            "timestamp": end_ts,
            "message": {
                "type": "StatusUpdate",
                "payload": {}
            }
        })
    )?;
    writeln!(
        wire_file,
        "{}",
        serde_json::json!({
            "timestamp": end_ts,
            "message": {
                "type": "TurnEnd",
                "payload": {}
            }
        })
    )?;

    let title = session_title(session)
        .chars()
        .take(TITLE_MAX_CHARS)
        .collect::<String>();
    let state = serde_json::json!({
        "version": 1,
        "approval": {
            "yolo": false,
            "auto_approve_actions": []
        },
        "additional_dirs": [],
        "custom_title": title,
        "title_generated": false,
        "title_generate_attempts": 0,
        "plan_mode": false,
        "plan_session_id": null,
        "plan_slug": null,
        "wire_mtime": null,
        "archived": false,
        "archived_at": null,
        "auto_archive_exempt": false,
        "todos": []
    });
    let mut state_file = File::create(&state_path)?;
    write!(state_file, "{}", serde_json::to_string_pretty(&state)?)?;
    wire_file.sync_all()?;
    context_file.sync_all()?;
    state_file.sync_all()?;

    if let Err(error) = register_exported_kimi_session(target_dir, &session_id) {
        let _ = std::fs::remove_dir_all(&session_dir);
        return Err(error);
    }
    Ok(session_id)
}

pub(super) fn block_to_kimi_content_part(block: &Block) -> Option<Value> {
    match block {
        Block::Text { text } => Some(serde_json::json!({
            "type": "text",
            "text": text
        })),
        Block::Thinking { text, .. } => Some(serde_json::json!({
            "type": "think",
            "think": text,
            "encrypted": null
        })),
        _ => visible_block_text(block).map(|text| {
            serde_json::json!({
                "type": "text",
                "text": text
            })
        }),
    }
}
