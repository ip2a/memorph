use crate::canonical::{CanonicalSession, EventRole, SessionEvent};
use crate::provider::{
    canonical_event_visible_message_role, canonical_event_visible_message_text,
    canonical_session_title, ProviderSourceMutation,
};
use crate::providers::cursor::db::{key_prefix_bounds, open_global_db};
use anyhow::{Context, Result};
use rusqlite::types::Value as SqliteValue;
use rusqlite::{params, OptionalExtension};
use serde_json::json;
use std::path::Path;
use uuid::Uuid;

#[cfg(test)]
static TEST_CURSOR_MUTATION_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<ProviderSourceMutation>>,
> = std::sync::OnceLock::new();

/// Build a minimal ProseMirror richText JSON for Cursor (new native format).
fn prosemirror_rich_text(text: &str) -> serde_json::Value {
    if text.is_empty() {
        json!({"type": "doc", "content": [{"type": "paragraph"}]})
    } else {
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": text}]
            }]
        })
    }
}

/// Generate a random base64 key matching Cursor's format.
fn random_base64_key() -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.r#gen();
    STANDARD.encode(&bytes)
}

/// Minimal bubble context matching Cursor's expected shape.
fn empty_bubble_context() -> serde_json::Value {
    json!({
        "composers": [],
        "selectedCommits": [],
        "selectedPullRequests": [],
        "selectedImages": [],
        "folderSelections": [],
        "fileSelections": [],
        "terminalFiles": [],
        "selections": [],
        "terminalSelections": [],
        "selectedDocs": [],
        "externalLinks": [],
        "cursorRules": [],
        "cursorCommands": [],
        "gitPRDiffSelections": [],
        "subagentSelections": [],
        "browserSelections": [],
        "extraContext": [],
        "mentions": {
            "composers": {},
            "selectedCommits": {},
            "selectedPullRequests": {},
            "gitDiff": [],
            "gitDiffFromBranchToMain": [],
            "selectedImages": {},
            "folderSelections": {},
            "fileSelections": {},
            "terminalFiles": {},
            "selections": {},
            "terminalSelections": {},
            "selectedDocs": {},
            "externalLinks": {},
            "diffHistory": [],
            "cursorRules": {},
            "cursorCommands": {},
            "uiElementSelections": [],
            "consoleLogs": [],
            "ideEditorsState": [],
            "gitPRDiffSelections": {},
            "subagentSelections": {},
            "browserSelections": {}
        }
    })
}

/// Delete a Cursor Composer session and all its bubbles.
pub fn delete_session(session_id: &str) -> Result<()> {
    let mut conn = open_global_db()?;
    let tx = conn.transaction()?;
    let bubbles_deleted = delete_session_with_conn(&tx, session_id)?;
    tx.commit()?;
    fail_cursor_mutation_after_database_write(ProviderSourceMutation::Delete)?;

    println!(
        "Deleted Cursor session {} ({} bubbles)",
        session_id, bubbles_deleted
    );
    Ok(())
}

pub fn delete_sessions(session_ids: &[&str]) -> Vec<Result<()>> {
    let mut conn = match open_global_db() {
        Ok(conn) => conn,
        Err(err) => {
            let message = err.to_string();
            return session_ids
                .iter()
                .map(|session_id| {
                    Err(anyhow::anyhow!(
                        "Failed to delete Cursor session {}: {}",
                        session_id,
                        message
                    ))
                })
                .collect();
        }
    };
    let tx = match conn.transaction() {
        Ok(tx) => tx,
        Err(err) => {
            let message = err.to_string();
            return session_ids
                .iter()
                .map(|session_id| {
                    Err(anyhow::anyhow!(
                        "Failed to delete Cursor session {}: {}",
                        session_id,
                        message
                    ))
                })
                .collect();
        }
    };

    let results: Vec<Result<()>> = session_ids
        .iter()
        .map(
            |session_id| match delete_session_with_conn(&tx, session_id) {
                Ok(bubbles_deleted) => {
                    println!(
                        "Deleted Cursor session {} ({} bubbles)",
                        session_id, bubbles_deleted
                    );
                    Ok(())
                }
                Err(err) => Err(err),
            },
        )
        .collect();

    if let Err(err) = tx.commit() {
        let message = err.to_string();
        return results
            .into_iter()
            .enumerate()
            .map(|(idx, result)| {
                result.and_then(|()| {
                    Err(anyhow::anyhow!(
                        "Failed to commit Cursor delete for session {}: {}",
                        session_ids[idx],
                        message
                    ))
                })
            })
            .collect();
    }

    results
        .into_iter()
        .map(|result| {
            result.and_then(|()| {
                fail_cursor_mutation_after_database_write(ProviderSourceMutation::Delete)
            })
        })
        .collect()
}

fn delete_session_with_conn(conn: &rusqlite::Connection, session_id: &str) -> Result<usize> {
    let bubble_prefix = format!("bubbleId:{}:", session_id);
    let (bubble_lower, bubble_upper) = key_prefix_bounds(&bubble_prefix);
    let bubbles_deleted = conn
        .execute(
            "DELETE FROM cursorDiskKV WHERE key >= ?1 AND key < ?2",
            params![bubble_lower, bubble_upper],
        )
        .with_context(|| format!("Failed to delete bubbles for composer {}", session_id))?;

    let composer_key = format!("composerData:{}", session_id);
    conn.execute("DELETE FROM cursorDiskKV WHERE key = ?1", [&composer_key])
        .with_context(|| format!("Failed to delete composer {}", session_id))?;

    conn.execute(
        "DELETE FROM composerHeaders WHERE composerId = ?1",
        [session_id],
    )
    .with_context(|| format!("Failed to delete composer header {}", session_id))?;

    Ok(bubbles_deleted)
}

/// Rename a current Cursor Composer session by updating only provider-owned name fields.
pub fn rename_session(session_id: &str, new_title: &str) -> Result<()> {
    let mut conn = open_global_db()?;
    let tx = conn.transaction()?;
    let composer_key = format!("composerData:{}", session_id);

    let header_updated = rename_json_value(
        &tx,
        "SELECT value FROM composerHeaders WHERE composerId = ?1",
        "UPDATE composerHeaders SET value = ?1 WHERE composerId = ?2",
        session_id,
        new_title,
        "composer header",
    )?;
    let composer_updated = rename_json_value(
        &tx,
        "SELECT value FROM cursorDiskKV WHERE key = ?1",
        "UPDATE cursorDiskKV SET value = ?1 WHERE key = ?2",
        &composer_key,
        new_title,
        "composer data",
    )?;
    anyhow::ensure!(
        header_updated || composer_updated,
        "Cursor composer not found: {session_id}"
    );

    tx.commit()?;
    fail_cursor_mutation_after_database_write(ProviderSourceMutation::Rename)?;
    Ok(())
}

fn rename_json_value(
    conn: &rusqlite::Connection,
    select_sql: &str,
    update_sql: &str,
    identity: &str,
    new_title: &str,
    row_name: &str,
) -> Result<bool> {
    let stored = conn
        .query_row(select_sql, [identity], |row| row.get::<_, SqliteValue>(0))
        .optional()
        .with_context(|| format!("Failed to read Cursor {row_name} {identity}"))?;
    let Some(stored) = stored else {
        return Ok(false);
    };
    let mut value = parse_json_value(&stored, row_name, identity)?;
    value
        .as_object_mut()
        .with_context(|| format!("Cursor {row_name} {identity} is not a JSON object"))?
        .insert("name".to_string(), json!(new_title));
    let updated = serialize_json_value(&stored, &value)?;
    conn.execute(update_sql, params![updated, identity])
        .with_context(|| format!("Failed to rename Cursor {row_name} {identity}"))?;
    Ok(true)
}

fn parse_json_value(
    stored: &SqliteValue,
    row_name: &str,
    identity: &str,
) -> Result<serde_json::Value> {
    match stored {
        SqliteValue::Text(text) => serde_json::from_str(text),
        SqliteValue::Blob(bytes) => serde_json::from_slice(bytes),
        _ => anyhow::bail!("Cursor {row_name} {identity} is not stored as TEXT or BLOB"),
    }
    .with_context(|| format!("Failed to parse Cursor {row_name} {identity}"))
}

fn serialize_json_value(template: &SqliteValue, value: &serde_json::Value) -> Result<SqliteValue> {
    match template {
        SqliteValue::Text(_) => Ok(SqliteValue::Text(serde_json::to_string(value)?)),
        SqliteValue::Blob(_) => Ok(SqliteValue::Blob(serde_json::to_vec(value)?)),
        _ => anyhow::bail!("Cursor JSON value is not stored as TEXT or BLOB"),
    }
}

#[cfg(test)]
pub(crate) fn set_test_cursor_mutation_failure(mutation: Option<ProviderSourceMutation>) {
    *TEST_CURSOR_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Cursor mutation failure lock") = mutation;
}

#[cfg(test)]
fn fail_cursor_mutation_after_database_write(mutation: ProviderSourceMutation) -> Result<()> {
    let mut failure = TEST_CURSOR_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Cursor mutation failure lock");
    if *failure == Some(mutation) {
        *failure = None;
        anyhow::bail!("injected Cursor mutation failure after database write");
    }
    Ok(())
}

#[cfg(not(test))]
fn fail_cursor_mutation_after_database_write(_mutation: ProviderSourceMutation) -> Result<()> {
    Ok(())
}

/// Build a native-format bubble JSON.
fn build_bubble_json(
    bubble_type: i32,
    bubble_id: &str,
    text: &str,
    created_at: &str,
    request_id: &str,
) -> serde_json::Value {
    json!({
        "_v": 3,
        "type": bubble_type,
        "bubbleId": bubble_id,
        "text": text,
        "richText": prosemirror_rich_text(text),
        "createdAt": created_at,
        "isAgentic": false,
        "skipRendering": false,
        "isNudge": false,
        "requestId": request_id,
        "checkpointId": Uuid::new_v4().to_string(),
        "modelInfo": { "modelName": "default" },
        "context": empty_bubble_context(),
        "conversationState": "~",
        "tokenCount": {
            "inputTokens": 0,
            "outputTokens": 0
        },
        "approximateLintErrors": [],
        "lints": [],
        "codebaseContextChunks": [],
        "commits": [],
        "pullRequests": [],
        "attachedCodeChunks": [],
        "assistantSuggestedDiffs": [],
        "gitDiffs": [],
        "interpreterResults": [],
        "images": [],
        "attachedFolders": [],
        "attachedFoldersNew": [],
        "userResponsesToSuggestedCodeBlocks": [],
        "suggestedCodeBlocks": [],
        "diffsForCompressingFiles": [],
        "relevantFiles": [],
        "toolResults": [],
        "notepads": [],
        "capabilities": [],
        "multiFileLinterErrors": [],
        "diffHistories": [],
        "recentLocationsHistory": [],
        "recentlyViewedFiles": [],
        "fileDiffTrajectories": [],
        "existedSubsequentTerminalCommand": false,
        "existedPreviousTerminalCommand": false,
        "docsReferences": [],
        "webReferences": [],
        "aiWebSearchResults": [],
        "attachedFoldersListDirResults": [],
        "humanChanges": [],
        "attachedHumanChanges": false,
        "summarizedComposers": [],
        "cursorRules": [],
        "cursorCommands": [],
        "cursorCommandsExplicitlySet": false,
        "pastChats": [],
        "pastChatsExplicitlySet": false,
        "contextPieces": [],
        "editTrailContexts": [],
        "allThinkingBlocks": [],
        "diffsSinceLastApply": [],
        "deletedFiles": [],
        "supportedTools": [],
        "attachedFileCodeChunksMetadataOnly": [],
        "consoleLogs": [],
        "uiElementPicked": [],
        "isRefunded": false,
        "knowledgeItems": [],
        "documentationSelections": [],
        "externalLinks": [],
        "projectLayouts": [],
        "mcpDescriptors": [],
        "workspaceUris": [],
        "capabilityContexts": [],
        "todos": [],
        "isPlanExecution": false
    })
}

pub fn export_session(session: &CanonicalSession, target_dir: &Path) -> Result<String> {
    let mut conn = open_global_db()?;
    let tx = conn.transaction()?;
    let composer_id = Uuid::new_v4().to_string();
    let workspace_id = Uuid::new_v4().to_string().replace("-", "");
    let workspace_path = target_dir.to_string_lossy().to_string();

    struct BubbleMeta {
        id: String,
        bubble_type: i32,
        text: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    }

    let bubbles: Vec<BubbleMeta> = session
        .events
        .iter()
        .filter_map(|event| {
            let role = canonical_event_visible_message_role(event)?;
            let text = cursor_bubble_text(event)?;
            Some(BubbleMeta {
                id: Uuid::new_v4().to_string(),
                bubble_type: if role == EventRole::User { 1 } else { 2 },
                text,
                timestamp: event.timestamp,
            })
        })
        .collect();

    let headers: Vec<serde_json::Value> = bubbles
        .iter()
        .map(|bubble| {
            json!({
                "bubbleId": bubble.id,
                "type": bubble.bubble_type
            })
        })
        .collect();

    let title = canonical_session_title(session);
    let first_active = bubbles
        .first()
        .map(|bubble| bubble.timestamp.timestamp_millis())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let last_active = bubbles
        .last()
        .map(|bubble| bubble.timestamp.timestamp_millis())
        .unwrap_or(first_active);

    let composer_data = json!({
        "_v": 14,
        "composerId": composer_id,
        "status": "completed",
        "text": "",
        "name": title,
        "richText": prosemirror_rich_text(""),
        "fullConversationHeadersOnly": headers,
        "conversationMap": {},
        "workspaceIdentifier": {
            "id": workspace_id,
            "uri": {
                "$mid": 1,
                "fsPath": workspace_path,
                "external": format!("file://{}", workspace_path),
                "path": workspace_path,
                "scheme": "file"
            }
        },
        "context": empty_bubble_context(),
        "createdAt": first_active,
        "lastUpdatedAt": last_active,
        "hasLoaded": true,
        "isAgentic": true,
        "agentBackend": "cursor-agent",
        "unifiedMode": "agent",
        "forceMode": "edit",
        "capabilities": [
            { "type": 30, "data": {} },
            { "type": 15, "data": { "bubbleDataMap": "{}" } },
            { "type": 22, "data": {} },
            { "type": 18, "data": {} },
            { "type": 19, "data": {} },
            { "type": 33, "data": {} },
            { "type": 32, "data": {} },
            { "type": 23, "data": {} },
            { "type": 16, "data": {} },
            { "type": 24, "data": {} },
            { "type": 21, "data": {} },
            { "type": 31, "data": {} },
            { "type": 29, "data": {} }
        ],
        "isFileListExpanded": false,
        "browserChipManuallyDisabled": false,
        "browserChipManuallyEnabled": false,
        "usageData": {},
        "allAttachedFileCodeChunksUris": [],
        "modelConfig": {
            "modelName": "default",
            "maxMode": false,
            "selectedModels": [{"modelId": "default", "parameters": []}]
        },
        "subComposerIds": [],
        "subagentComposerIds": [],
        "capabilityContexts": [],
        "todos": [],
        "isQueueExpanded": true,
        "hasUnreadMessages": false,
        "gitHubPromptDismissed": false,
        "totalLinesAdded": 0,
        "totalLinesRemoved": 0,
        "addedFiles": 0,
        "removedFiles": 0,
        "isDraft": false,
        "isCreatingWorktree": false,
        "isApplyingWorktree": false,
        "isUndoingWorktree": false,
        "applied": false,
        "pendingCreateWorktree": false,
        "worktreeStartedReadOnly": false,
        "isBestOfNSubcomposer": false,
        "isBestOfNParent": false,
        "bestOfNJudgeWinner": false,
        "isSpec": false,
        "isProject": false,
        "isSpecSubagentDone": false,
        "isContinuationInProgress": false,
        "stopHookLoopCount": 0,
        "branches": [],
        "speculativeSummarizationEncryptionKey": random_base64_key(),
        "isNAL": true,
        "planModeSuggestionUsed": false,
        "debugModeSuggestionUsed": false,
        "conversationState": "~",
        "queueItems": [],
        "blobEncryptionKey": random_base64_key(),
        "latestChatGenerationUUID": Uuid::new_v4().to_string(),
        "subtitle": "",
        "filesChangedCount": 0,
        "glassMetaParentAgent": false,
        "restrictAgentModeSwitching": false,
        "applyAgentBackendTypeRestrictions": false
    });

    let composer_key = format!("composerData:{}", composer_id);
    tx.execute(
        "INSERT OR REPLACE INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
        (&composer_key, composer_data.to_string().as_bytes()),
    )
    .context("Failed to insert composer data")?;

    for (idx, bubble) in bubbles.iter().enumerate() {
        let request_id = if bubble.bubble_type == 1 {
            Uuid::new_v4().to_string()
        } else {
            String::new()
        };
        let bubble_data = build_bubble_json(
            bubble.bubble_type,
            &bubble.id,
            &bubble.text,
            &bubble.timestamp.to_rfc3339(),
            &request_id,
        );
        let bubble_key = format!("bubbleId:{}:{}", composer_id, bubble.id);
        tx.execute(
            "INSERT OR REPLACE INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            (&bubble_key, bubble_data.to_string().as_bytes()),
        )
        .with_context(|| format!("Failed to insert bubble {}", idx))?;
    }

    let header = json!({
        "composerId": composer_id,
        "name": title,
        "subtitle": "",
        "createdAt": first_active,
        "lastUpdatedAt": last_active,
        "workspaceIdentifier": {
            "id": workspace_id,
            "uri": {
                "$mid": 1,
                "fsPath": workspace_path,
                "external": format!("file://{}", workspace_path),
                "path": workspace_path,
                "scheme": "file"
            }
        }
    });
    tx.execute(
        "INSERT INTO composerHeaders
         (composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent,
          recency, checkpointAt, value)
         VALUES (?1, ?2, ?3, ?4, 0, 0, ?4, NULL, ?5)",
        params![
            &composer_id,
            &workspace_id,
            first_active,
            last_active,
            serde_json::to_string(&header)?,
        ],
    )
    .context("Failed to insert current Cursor composer header")?;
    tx.commit()?;
    let _ = conn.execute("PRAGMA wal_checkpoint(PASSIVE)", []);

    Ok(composer_id)
}

fn cursor_bubble_text(event: &SessionEvent) -> Option<String> {
    canonical_event_visible_message_text(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{
        EventBlock, EventLinks, EventMetadata, EventSource, MappingDisposition, SessionEventKind,
    };
    use chrono::Utc;
    use std::collections::BTreeMap;

    #[test]
    fn compressed_segment_exports_as_portable_cursor_bubble_text() {
        let event = SessionEvent {
            id: "compressed-source".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::Assistant,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Compressed {
                source_provider_id: "opencode".to_string(),
                summary: "compressed summary".to_string(),
                source_event_ids: vec![
                    "old-event-1".to_string(),
                    "old-event-2".to_string(),
                    "old-event-3".to_string(),
                ],
                source_event_count: None,
                archive_ref: Some("memorph-archive://s1/archive.json.gz".to_string()),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "memorph".to_string(),
                    original_id: None,
                    original_role: Some("assistant".to_string()),
                    phase: Some("compression".to_string()),
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Normalized,
                provider_ext: BTreeMap::new(),
            },
        };

        let text = cursor_bubble_text(&event).expect("visible compressed bubble text");

        assert!(text.contains("[Compressed session segment from opencode]"));
        assert!(text.contains("compressed summary"));
        assert!(text.contains("Source event count: 3"));
        assert!(text.contains("Archive: memorph-archive://s1/archive.json.gz"));
        assert!(text.contains("memorph compression retrieve memorph-archive://s1/archive.json.gz --query <terms> --max-results 5"));
        assert!(!text.contains("old-event-1"));
        assert!(!text.contains("old-event-2"));
        assert!(!text.contains("old-event-3"));
    }

    #[test]
    fn internal_events_do_not_export_as_cursor_bubble_text() {
        let event = SessionEvent {
            id: "internal".to_string(),
            kind: SessionEventKind::Lifecycle,
            role: EventRole::System,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Text {
                text: "internal context".to_string(),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "codex".to_string(),
                    original_id: None,
                    original_role: Some("user".to_string()),
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Normalized,
                provider_ext: BTreeMap::new(),
            },
        };

        assert!(cursor_bubble_text(&event).is_none());
    }
}
