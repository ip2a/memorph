use super::*;

pub(super) fn delete_opencode_session(session_id: &str) -> Result<()> {
    let db_path = get_db_path();
    let database_message_ids = if db_path.exists() {
        let conn = Connection::open(&db_path)?;
        opencode_message_ids(&conn, session_id)?
    } else {
        HashSet::new()
    };
    let mutation_paths = discover_opencode_mutation_paths(session_id, &database_message_ids)?;
    let mut database_deleted = false;

    if db_path.exists() {
        let mut conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let tx = conn.transaction()?;
        database_deleted = tx.execute("DELETE FROM session WHERE id = ?1", [session_id])? > 0;
        tx.commit()?;
    }
    let filesystem_present = !mutation_paths.session_files.is_empty()
        || path_lexists(&mutation_paths.message_dir)
        || !mutation_paths.part_dirs.is_empty();
    if !database_deleted && !filesystem_present {
        anyhow::bail!("OpenCode session not found: {session_id}");
    }

    fail_opencode_mutation_after_database_write(ProviderSourceMutation::Delete)?;
    for session_path in &mutation_paths.session_files {
        remove_opencode_filesystem_entry(session_path)?;
        if let Some(parent) = session_path.parent() {
            match std::fs::remove_dir(parent) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    remove_opencode_filesystem_entry(&mutation_paths.message_dir)?;
    for part_dir in &mutation_paths.part_dirs {
        remove_opencode_filesystem_entry(part_dir)?;
    }
    Ok(())
}

pub(super) fn replace_opencode_session(session_id: &str, session: &Session) -> Result<()> {
    let db_path = get_db_path();
    if !db_path.exists() {
        anyhow::bail!("OpenCode database does not exist: {}", db_path.display());
    }
    let mut conn = Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    let (project_id, slug, directory, created_at): (String, String, String, i64) = conn
        .query_row(
            "SELECT project_id, slug, directory, time_created FROM session WHERE id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .with_context(|| format!("OpenCode session not found: {session_id}"))?;
    let old_message_ids = opencode_message_ids(&conn, session_id)?;
    let old_paths = discover_opencode_mutation_paths(session_id, &old_message_ids)?;
    let now = Utc::now().timestamp_millis();
    let title = session_title(session);
    let projection = build_opencode_projection(OpenCodeProjectionInput {
        session,
        session_id,
        project_id: &project_id,
        slug: &slug,
        target_dir: &directory,
        title: &title,
        created_at,
        updated_at: now,
    });

    let tx = conn.transaction()?;
    tx.execute("DELETE FROM part WHERE session_id = ?1", [session_id])?;
    tx.execute("DELETE FROM message WHERE session_id = ?1", [session_id])?;
    tx.execute(
        "UPDATE session SET title = ?1, time_updated = ?2 WHERE id = ?3",
        rusqlite::params![title, now, session_id],
    )?;
    insert_opencode_projection_rows(&tx, session_id, &projection.messages, &projection.parts)?;
    tx.commit()?;

    fail_opencode_mutation_after_database_write(ProviderSourceMutation::Replace)?;
    for path in old_paths.session_files {
        remove_opencode_filesystem_entry(&path)?;
    }
    remove_opencode_filesystem_entry(&old_paths.message_dir)?;
    for path in old_paths.part_dirs {
        remove_opencode_filesystem_entry(&path)?;
    }
    write_to_filesystem(
        session_id,
        &project_id,
        &projection.session_json,
        &projection.messages,
        &projection.parts,
    )?;

    let source = opencode_db_session_source_locator(session_id);
    let imported = import_canonical_session_from_source(session_id, &source)?;
    if imported.session.identity.id != session_id {
        anyhow::bail!("OpenCode replacement validation changed session identity");
    }
    Ok(())
}

pub(super) fn rename_opencode_session(session_id: &str, new_title: &str) -> Result<()> {
    let db_path = get_db_path();
    let session_files = find_opencode_session_files(session_id)?;
    let now = Utc::now().timestamp_millis();
    let mut database_updated = false;

    if db_path.exists() {
        let mut conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let tx = conn.transaction()?;
        database_updated = tx.execute(
            "UPDATE session SET title = ?1, time_updated = ?2 WHERE id = ?3",
            rusqlite::params![new_title, now, session_id],
        )? > 0;
        tx.commit()?;
    }
    if !database_updated && session_files.is_empty() {
        anyhow::bail!("OpenCode session not found: {session_id}");
    }

    fail_opencode_mutation_after_database_write(ProviderSourceMutation::Rename)?;
    for path in session_files {
        let content = std::fs::read(&path)
            .with_context(|| format!("Failed to read OpenCode session file: {}", path.display()))?;
        let mut value: Value = serde_json::from_slice(&content).with_context(|| {
            format!("Failed to parse OpenCode session file: {}", path.display())
        })?;
        let object = value.as_object_mut().with_context(|| {
            format!("OpenCode session file is not an object: {}", path.display())
        })?;
        object.insert("title".to_string(), Value::String(new_title.to_string()));
        let time = object
            .get_mut("time")
            .and_then(Value::as_object_mut)
            .with_context(|| {
                format!(
                    "OpenCode session file has no time object: {}",
                    path.display()
                )
            })?;
        time.insert("updated".to_string(), Value::Number(now.into()));
        std::fs::write(&path, serde_json::to_vec_pretty(&value)?).with_context(|| {
            format!("Failed to write OpenCode session file: {}", path.display())
        })?;
    }
    Ok(())
}
