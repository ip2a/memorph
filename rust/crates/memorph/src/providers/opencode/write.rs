use super::*;

pub(super) fn generate_opencode_id(prefix: &str) -> String {
    let uuid = Uuid::new_v4().to_string().replace("-", "");
    format!("{}_{}", prefix, &uuid[..24.min(uuid.len())])
}

pub(super) fn generate_slug() -> String {
    let adjectives = [
        "bright", "calm", "swift", "keen", "bold", "warm", "cool", "sharp", "clear", "steady",
    ];
    let nouns = [
        "river", "forest", "mountain", "ocean", "sky", "star", "path", "garden", "valley",
        "horizon",
    ];
    let idx1 = (Uuid::new_v4().as_u128() % adjectives.len() as u128) as usize;
    let idx2 = (Uuid::new_v4().as_u128() % nouns.len() as u128) as usize;
    format!("{}-{}", adjectives[idx1], nouns[idx2])
}

pub(super) struct OpenCodeProjection {
    pub(super) session_json: Value,
    pub(super) messages: Vec<(String, i64, Value)>,
    pub(super) parts: Vec<(String, String, i64, Value)>,
}

pub(super) fn export_canonical_session(session: &Session, target_dir: &Path) -> Result<String> {
    let now = Utc::now().timestamp_millis();
    let session_id = generate_opencode_id("ses");
    let project_id = find_or_create_project(target_dir)?;
    let slug = generate_slug();
    let target_dir_str = target_dir.to_string_lossy().to_string();
    let title = session_title(session);
    let projection = build_opencode_projection(OpenCodeProjectionInput {
        session,
        session_id: &session_id,
        project_id: &project_id,
        slug: &slug,
        target_dir: &target_dir_str,
        title: &title,
        created_at: now,
        updated_at: now,
    });

    write_to_db(OpenCodeDatabaseWrite {
        session_id: &session_id,
        project_id: &project_id,
        slug: &slug,
        directory: &target_dir_str,
        title: &title,
        now,
        messages: &projection.messages,
        parts: &projection.parts,
    })
    .context("Failed to write to OpenCode SQLite database")?;
    load_session_from_db(&session_id).context("Failed to verify OpenCode SQLite write result")?;
    write_to_filesystem(
        &session_id,
        &project_id,
        &projection.session_json,
        &projection.messages,
        &projection.parts,
    )?;

    Ok(session_id)
}

pub(super) struct OpenCodeProjectionInput<'a> {
    pub(super) session: &'a Session,
    pub(super) session_id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) slug: &'a str,
    pub(super) target_dir: &'a str,
    pub(super) title: &'a str,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

pub(super) fn build_opencode_projection(input: OpenCodeProjectionInput<'_>) -> OpenCodeProjection {
    let OpenCodeProjectionInput {
        session,
        session_id,
        project_id,
        slug,
        target_dir,
        title,
        created_at,
        updated_at,
    } = input;
    let session_json = serde_json::json!({
        "id": session_id,
        "slug": slug,
        "version": OPENCODE_VERSION,
        "projectID": project_id,
        "directory": target_dir,
        "title": title,
        "time": {
            "created": created_at,
            "updated": updated_at
        }
    });

    let mut oc_messages: Vec<(String, i64, Value)> = Vec::new();
    let mut oc_parts: Vec<(String, String, i64, Value)> = Vec::new();
    let mut last_user_msg_id: Option<String> = None;

    for event in &session.events {
        if let Some(segment) = compression::compressed_segment(event) {
            append_compressed_opencode_segment(
                session_id,
                event,
                segment,
                target_dir,
                &mut last_user_msg_id,
                &mut oc_messages,
                &mut oc_parts,
            );
            continue;
        }

        let Some(visible_role) = event_visible_message_role(event) else {
            continue;
        };
        if !event_is_visible_message(event) {
            continue;
        }
        let msg_id = generate_opencode_id("msg");
        let msg_created = event.timestamp.timestamp_millis();
        let (role, parent_id) = match visible_role {
            Role::Assistant => ("assistant", last_user_msg_id.clone()),
            Role::User => {
                last_user_msg_id = Some(msg_id.clone());
                ("user", None)
            }
            _ => {
                last_user_msg_id = Some(msg_id.clone());
                ("user", None)
            }
        };

        let msg_json = build_opencode_message_data_from_event(
            session_id,
            event,
            &msg_id,
            role,
            parent_id.as_deref(),
            target_dir,
        );
        oc_messages.push((msg_id.clone(), msg_created, msg_json));

        for block in &event.blocks {
            let part_id = generate_opencode_id("prt");
            let part_created = msg_created + 1;
            let Some(part_json) =
                block_to_opencode_part(session_id, &msg_id, &part_id, block, part_created)
            else {
                continue;
            };
            oc_parts.push((part_id, msg_id.clone(), part_created, part_json));
        }
    }

    OpenCodeProjection {
        session_json,
        messages: oc_messages,
        parts: oc_parts,
    }
}

pub(super) fn append_compressed_opencode_segment(
    session_id: &str,
    event: &Event,
    segment: CompressedSegment,
    target_dir: &str,
    last_user_msg_id: &mut Option<String>,
    oc_messages: &mut Vec<(String, i64, Value)>,
    oc_parts: &mut Vec<(String, String, i64, Value)>,
) {
    let created = event.timestamp.timestamp_millis();
    let marker_msg_id = generate_opencode_id("msg");
    let summary_msg_id = generate_opencode_id("msg");
    let marker_part_id = generate_opencode_id("prt");
    let summary_part_id = generate_opencode_id("prt");

    let mut marker_msg = build_opencode_message_data_from_event(
        session_id,
        event,
        &marker_msg_id,
        "user",
        None,
        target_dir,
    );
    if let Some(obj) = marker_msg.as_object_mut() {
        obj.insert("mode".to_string(), Value::String("compaction".to_string()));
        obj.insert("agent".to_string(), Value::String("compaction".to_string()));
    }
    oc_messages.push((marker_msg_id.clone(), created, marker_msg));
    let marker_part = opencode_compaction_part(
        session_id,
        &marker_msg_id,
        &marker_part_id,
        &segment.source_provider_id,
        &segment.source_event_ids,
        segment.source_event_count,
        segment.archive_ref.as_deref(),
    );
    oc_parts.push((
        marker_part_id,
        marker_msg_id.clone(),
        created + 1,
        marker_part,
    ));

    let mut summary_msg = build_opencode_message_data_from_event(
        session_id,
        event,
        &summary_msg_id,
        "assistant",
        Some(&marker_msg_id),
        target_dir,
    );
    if let Some(obj) = summary_msg.as_object_mut() {
        obj.insert("summary".to_string(), Value::Bool(true));
        obj.insert("mode".to_string(), Value::String("compaction".to_string()));
        obj.insert("agent".to_string(), Value::String("compaction".to_string()));
    }
    oc_messages.push((summary_msg_id.clone(), created + 2, summary_msg));
    oc_parts.push((
        summary_part_id.clone(),
        summary_msg_id.clone(),
        created + 3,
        serde_json::json!({
            "id": summary_part_id,
            "sessionID": session_id,
            "messageID": summary_msg_id,
            "type": "text",
            "text": segment.summary,
        }),
    ));

    *last_user_msg_id = Some(marker_msg_id);
}

pub(super) fn opencode_compaction_part(
    session_id: &str,
    msg_id: &str,
    part_id: &str,
    source_provider_id: &str,
    source_event_ids: &[String],
    source_event_count: Option<usize>,
    archive_ref: Option<&str>,
) -> Value {
    let mut part = serde_json::json!({
        "id": part_id,
        "sessionID": session_id,
        "messageID": msg_id,
        "type": "compaction",
        "auto": false,
        "memorph": {
            "sourceProviderID": source_provider_id,
            "sourceEventIDs": source_event_ids,
            "sourceEventCount": source_event_count,
        }
    });
    if let Some(archive_ref) = archive_ref {
        part["memorph"]["archiveRef"] = Value::String(archive_ref.to_string());
        part["memorph"]["retrievalHint"] = Value::String(compression_retrieval_hint(archive_ref));
    }
    part
}

pub(super) fn build_opencode_message_data_from_event(
    session_id: &str,
    event: &Event,
    msg_id: &str,
    role: &str,
    parent_id: Option<&str>,
    target_dir: &str,
) -> Value {
    let model_name = event
        .metadata
        .model
        .as_deref()
        .filter(|value| !value.is_empty());
    let provider_id = model_name
        .map(provider_for_model)
        .unwrap_or("openai")
        .to_string();
    let model_id = model_name
        .map(ToString::to_string)
        .unwrap_or_else(|| default_model_id(&provider_id).to_string());
    let mut msg_json = serde_json::Map::new();
    msg_json.insert("id".to_string(), Value::String(msg_id.to_string()));
    msg_json.insert(
        "sessionID".to_string(),
        Value::String(session_id.to_string()),
    );
    msg_json.insert("role".to_string(), Value::String(role.to_string()));
    msg_json.insert(
        "time".to_string(),
        serde_json::json!({"created": event.timestamp.timestamp_millis()}),
    );
    if let Some(parent_id) = parent_id {
        msg_json.insert("parentID".to_string(), Value::String(parent_id.to_string()));
    }
    msg_json.insert("providerID".to_string(), Value::String(provider_id.clone()));
    msg_json.insert("modelID".to_string(), Value::String(model_id.to_string()));
    msg_json.insert(
        "model".to_string(),
        serde_json::json!({
            "providerID": provider_id,
            "modelID": model_id,
        }),
    );
    msg_json.insert("agent".to_string(), Value::String("build".to_string()));
    msg_json.insert("mode".to_string(), Value::String("build".to_string()));
    msg_json.insert(
        "tokens".to_string(),
        serde_json::json!({
            "input": event.metadata.usage.as_ref().and_then(|usage| usage.input_tokens).unwrap_or(0),
            "output": event.metadata.usage.as_ref().and_then(|usage| usage.output_tokens).unwrap_or(0),
            "reasoning": 0,
            "cache": {"read": 0, "write": 0},
        }),
    );
    if role == "assistant" {
        msg_json.insert(
            "path".to_string(),
            serde_json::json!({"cwd": target_dir, "root": target_dir}),
        );
        msg_json.insert("cost".to_string(), Value::from(0));
        msg_json.insert("finish".to_string(), Value::String("stop".to_string()));
    }
    Value::Object(msg_json)
}

pub(super) fn block_to_opencode_part(
    session_id: &str,
    msg_id: &str,
    part_id: &str,
    block: &Block,
    part_created: i64,
) -> Option<Value> {
    match block {
        Block::Text { text } => Some(serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "text",
            "text": text,
        })),
        Block::Thinking { text, .. } => Some(serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "reasoning",
            "text": text,
        })),
        Block::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => Some(serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "tool",
            "callID": tool_call_id,
            "tool": "unknown",
            "state": {
                "status": if *is_error { "error" } else { "completed" },
                "input": {},
                "output": content,
                "title": "Tool result",
                "metadata": {},
                "time": {
                    "start": part_created,
                    "end": part_created
                }
            }
        })),
        Block::Image {
            mime_type, data, ..
        } => Some(serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "file",
            "mime": mime_type,
            "filename": "image.png",
            "url": data.as_deref().unwrap_or(""),
        })),
        Block::File { path, content, .. } => Some(serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "file",
            "mime": "text/plain",
            "filename": path,
            "url": content.as_deref().unwrap_or(""),
        })),
        _ => visible_block_text(block).map(|text| {
            serde_json::json!({
                "id": part_id,
                "sessionID": session_id,
                "messageID": msg_id,
                "type": "text",
                "text": text,
            })
        }),
    }
}

pub(super) fn parse_data_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

/// Derive the OpenCode model provider from a canonical model name.
fn provider_for_model(model: &str) -> &'static str {
    let model = model.to_ascii_lowercase();
    if model.starts_with("claude") {
        "anthropic"
    } else if model.starts_with("gpt")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.starts_with("o5")
    {
        "openai"
    } else if model.starts_with("gemini") {
        "google"
    } else if model.starts_with("deepseek") {
        "deepseek"
    } else if model.starts_with("grok") {
        "x-ai"
    } else if model.starts_with("qwen") || model.starts_with("qwq") {
        "qwen"
    } else if model.starts_with("mistral") || model.starts_with("codestral") {
        "mistral"
    } else {
        "openai"
    }
}

pub(super) fn default_model_id(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-sonnet-4-5",
        _ => "gpt-5.3-codex",
    }
}

// ---------------------------------------------------------------------------
// DB operations
// ---------------------------------------------------------------------------

pub(super) fn find_or_create_project(target_dir: &Path) -> Result<String> {
    let db_path = get_db_path();
    if !db_path.exists() {
        // Generate a deterministic project ID from path
        return Ok(generate_project_id(target_dir));
    }

    let conn = Connection::open(&db_path)?;
    let target_dir_str = target_dir.to_string_lossy().to_string();

    // Try to find existing project
    let existing: Result<String, _> = conn.query_row(
        "SELECT id FROM project WHERE worktree = ?1 ORDER BY time_updated DESC LIMIT 1",
        [&target_dir_str],
        |row| row.get(0),
    );

    if let Ok(id) = existing {
        return Ok(id);
    }

    // Create new project
    let project_id = generate_project_id(target_dir);
    let now = Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO project (id, worktree, vcs, name, time_created, time_updated, sandboxes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        [
            &project_id,
            &target_dir_str,
            &get_git_remote(target_dir).unwrap_or_default(),
            target_dir.file_name().and_then(|n| n.to_str()).unwrap_or("project"),
            &now.to_string(),
            &now.to_string(),
            "{}",
        ],
    )?;

    Ok(project_id)
}

pub(super) fn generate_project_id(target_dir: &Path) -> String {
    // Use a SHA256 of the absolute path for determinism
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let path_str = target_dir.to_string_lossy().to_string();
    let mut hasher = DefaultHasher::new();
    path_str.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{:016x}{:024x}", hash, hash)
}

pub(super) fn get_git_remote(dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

pub(super) struct OpenCodeDatabaseWrite<'a> {
    pub(super) session_id: &'a str,
    pub(super) project_id: &'a str,
    pub(super) slug: &'a str,
    pub(super) directory: &'a str,
    pub(super) title: &'a str,
    pub(super) now: i64,
    pub(super) messages: &'a [(String, i64, Value)],
    pub(super) parts: &'a [(String, String, i64, Value)],
}

pub(super) fn write_to_db(input: OpenCodeDatabaseWrite<'_>) -> Result<()> {
    let OpenCodeDatabaseWrite {
        session_id,
        project_id,
        slug,
        directory,
        title,
        now,
        messages,
        parts,
    } = input;
    let db_path = get_db_path();
    if !db_path.exists() {
        anyhow::bail!(
            "OpenCode database does not exist: {}. Please launch OpenCode once to initialize storage before importing.",
            db_path.display()
        );
    }
    let mut conn = Connection::open(&db_path)?;

    let tx = conn.transaction()?;

    // Insert session
    tx.execute(
        "INSERT INTO session (id, project_id, slug, directory, title, version, time_created, time_updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        [session_id, project_id, slug, directory, title, OPENCODE_VERSION, &now.to_string(), &now.to_string()],
    )?;

    insert_opencode_projection_rows(&tx, session_id, messages, parts)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn insert_opencode_projection_rows(
    tx: &rusqlite::Transaction<'_>,
    session_id: &str,
    messages: &[(String, i64, Value)],
    parts: &[(String, String, i64, Value)],
) -> Result<()> {
    for (msg_id, created, data) in messages {
        let mut db_data = data.clone();
        if let Value::Object(ref mut map) = db_data {
            map.remove("id");
            map.remove("sessionID");
        }
        let data_str = serde_json::to_string(&db_data)?;
        tx.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![msg_id, session_id, created, created, data_str],
        )?;
    }
    for (part_id, msg_id, created, data) in parts {
        let mut db_data = data.clone();
        if let Value::Object(ref mut map) = db_data {
            map.remove("id");
            map.remove("messageID");
            map.remove("sessionID");
        }
        let data_str = serde_json::to_string(&db_data)?;
        tx.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![part_id, msg_id, session_id, created, created, data_str],
        )?;
    }
    Ok(())
}

pub(super) fn write_opencode_json_atomic(path: &Path, value: &Value) -> Result<()> {
    let temp_path = path.with_extension(format!("json.memorph-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = File::create(&temp_path)?;
        serde_json::to_writer_pretty(&mut file, value)?;
        use std::io::Write as _;
        file.flush()?;
        file.sync_all()?;
        std::fs::rename(&temp_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp_path);
    }
    result
}

pub(super) fn write_to_filesystem(
    session_id: &str,
    project_id: &str,
    session_json: &Value,
    messages: &[(String, i64, Value)],
    parts: &[(String, String, i64, Value)],
) -> Result<()> {
    let storage_dir = get_opencode_dir().join("storage");

    // Write session
    let session_dir = storage_dir.join("session").join(project_id);
    std::fs::create_dir_all(&session_dir)?;
    let session_file = session_dir.join(format!("{}.json", session_id));
    write_opencode_json_atomic(&session_file, session_json)?;

    // Write messages
    let msg_dir = storage_dir.join("message").join(session_id);
    std::fs::create_dir_all(&msg_dir)?;
    for (msg_id, _created, data) in messages {
        let msg_file = msg_dir.join(format!("{}.json", msg_id));
        write_opencode_json_atomic(&msg_file, data)?;
    }

    // Write parts
    let parts_base = storage_dir.join("part");
    for (part_id, msg_id, _created, data) in parts {
        let part_dir = parts_base.join(msg_id);
        std::fs::create_dir_all(&part_dir)?;
        let part_file = part_dir.join(format!("{}.json", part_id));
        write_opencode_json_atomic(&part_file, data)?;
    }

    Ok(())
}
