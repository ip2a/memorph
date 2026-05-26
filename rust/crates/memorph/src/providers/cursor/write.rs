use crate::canonical::{CanonicalSession, EventRole, SessionEvent};
use crate::provider::{canonical_event_text, canonical_session_title};
use crate::providers::cursor::db::{key_prefix_bounds, open_global_db};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::json;
use std::path::Path;
use uuid::Uuid;

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

/// Read the current composer.composerHeaders index from ItemTable.
fn read_composer_index(conn: &rusqlite::Connection) -> Result<serde_json::Value> {
    let json_str: String = conn
        .query_row(
            "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = 'composer.composerHeaders'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| r#"{"allComposers": []}"#.to_string());
    Ok(serde_json::from_str(&json_str)?)
}

/// Write the composer.composerHeaders index back to ItemTable.
fn write_composer_index(conn: &rusqlite::Connection, index: &serde_json::Value) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
        (
            "composer.composerHeaders",
            serde_json::to_string(index)?.as_bytes(),
        ),
    )?;
    Ok(())
}

/// Add or update a composer entry in the composer.composerHeaders index.
fn upsert_composer_index(
    conn: &rusqlite::Connection,
    composer_id: &str,
    name: &str,
    subtitle: &str,
    workspace_id: &str,
    workspace_path: &str,
    created_at: i64,
    last_updated_at: i64,
) -> Result<()> {
    let mut index = read_composer_index(conn)?;
    let all_composers = index
        .get_mut("allComposers")
        .and_then(|v| v.as_array_mut())
        .context("Invalid composer.composerHeaders format")?;

    // Remove existing entry if present
    all_composers.retain(|c| c.get("composerId").and_then(|v| v.as_str()) != Some(composer_id));

    let entry = json!({
        "type": "head",
        "composerId": composer_id,
        "name": name,
        "subtitle": subtitle,
        "lastUpdatedAt": last_updated_at,
        "createdAt": created_at,
        "unifiedMode": "agent",
        "forceMode": "edit",
        "hasUnreadMessages": false,
        "totalLinesAdded": 0,
        "totalLinesRemoved": 0,
        "filesChangedCount": 0,
        "hasBlockingPendingActions": false,
        "isArchived": false,
        "isDraft": false,
        "isWorktree": false,
        "worktreeStartedReadOnly": false,
        "isSpec": false,
        "isProject": false,
        "glassMetaParentAgent": false,
        "isBestOfNSubcomposer": false,
        "numSubComposers": 0,
        "referencedPlans": [],
        "branches": [],
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

    all_composers.insert(0, entry);
    write_composer_index(conn, &index)?;
    Ok(())
}

/// Remove a composer from the composer.composerHeaders index.
fn remove_composer_index(conn: &rusqlite::Connection, composer_id: &str) -> Result<()> {
    let mut index = read_composer_index(conn)?;
    if let Some(all_composers) = index.get_mut("allComposers").and_then(|v| v.as_array_mut()) {
        all_composers.retain(|c| c.get("composerId").and_then(|v| v.as_str()) != Some(composer_id));
        write_composer_index(conn, &index)?;
    }
    Ok(())
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
    let conn = open_global_db()?;
    let bubbles_deleted = delete_session_with_conn(&conn, session_id)?;

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

    let _ = remove_composer_index(conn, session_id);

    Ok(bubbles_deleted)
}

/// Rename a Cursor Composer session by updating its name field.
pub fn rename_session(session_id: &str, new_title: &str) -> Result<()> {
    let conn = open_global_db()?;
    let composer_key = format!("composerData:{}", session_id);

    // Read existing composer data
    let existing = conn
        .query_row(
            "SELECT CAST(value AS TEXT) FROM cursorDiskKV WHERE key = ?1",
            [&composer_key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("Failed to read composer {}", session_id))?;

    let existing_json = match existing {
        Some(s) => s,
        None => anyhow::bail!("Cursor composer not found: {}", session_id),
    };

    let mut composer_json = serde_json::from_str::<serde_json::Value>(&existing_json)
        .with_context(|| format!("Failed to parse composer {} JSON", session_id))?;

    // Update the name field (title)
    if let Some(obj) = composer_json.as_object_mut() {
        obj.insert("name".to_string(), json!(new_title));
    }

    // Write back
    let updated_json = serde_json::to_string(&composer_json)?;
    conn.execute(
        "INSERT OR REPLACE INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
        (&composer_key, updated_json.as_bytes()),
    )
    .with_context(|| format!("Failed to rename composer {}", session_id))?;

    // Update index
    let mut index = read_composer_index(&conn)?;
    if let Some(all_composers) = index.get_mut("allComposers").and_then(|v| v.as_array_mut()) {
        for c in all_composers.iter_mut() {
            if c.get("composerId").and_then(|v| v.as_str()) == Some(session_id) {
                c.as_object_mut()
                    .map(|o| o.insert("name".to_string(), json!(new_title)));
                break;
            }
        }
        let _ = write_composer_index(&conn, &index);
    }

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
    let conn = open_global_db()?;
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
            let text = cursor_bubble_text(event);
            if text.trim().is_empty() {
                return None;
            }
            Some(BubbleMeta {
                id: Uuid::new_v4().to_string(),
                bubble_type: if event.role == EventRole::User { 1 } else { 2 },
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
    conn.execute(
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
        conn.execute(
            "INSERT OR REPLACE INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            (&bubble_key, bubble_data.to_string().as_bytes()),
        )
        .with_context(|| format!("Failed to insert bubble {}", idx))?;
    }

    let _ = upsert_composer_index(
        &conn,
        &composer_id,
        &title,
        "",
        &workspace_id,
        &workspace_path,
        first_active,
        last_active,
    );
    let _ = conn.execute("PRAGMA wal_checkpoint(PASSIVE)", []);

    Ok(composer_id)
}

fn cursor_bubble_text(event: &SessionEvent) -> String {
    canonical_event_text(event)
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

        let text = cursor_bubble_text(&event);

        assert!(text.contains("[Compressed session segment from opencode]"));
        assert!(text.contains("compressed summary"));
        assert!(text.contains("Source event count: 3"));
        assert!(text.contains("Archive: memorph-archive://s1/archive.json.gz"));
        assert!(!text.contains("old-event-1"));
        assert!(!text.contains("old-event-2"));
        assert!(!text.contains("old-event-3"));
    }
}
