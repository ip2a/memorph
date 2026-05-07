use crate::model::MemorphSession;
use crate::providers::cursor::db::open_global_db;
use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
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
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.gen();
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
        ("composer.composerHeaders", serde_json::to_string(index)?.as_bytes()),
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

    // Delete all bubbles for this composer
    let bubble_pattern = format!("bubbleId:{}:%", session_id);
    let bubbles_deleted = conn
        .execute(
            "DELETE FROM cursorDiskKV WHERE key LIKE ?1",
            [&bubble_pattern],
        )
        .with_context(|| format!("Failed to delete bubbles for composer {}", session_id))?;

    // Delete the composer metadata
    let composer_key = format!("composerData:{}", session_id);
    conn.execute("DELETE FROM cursorDiskKV WHERE key = ?1", [&composer_key])
        .with_context(|| format!("Failed to delete composer {}", session_id))?;

    // Remove from index
    let _ = remove_composer_index(&conn, session_id);

    println!(
        "Deleted Cursor session {} ({} bubbles)",
        session_id, bubbles_deleted
    );
    Ok(())
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
                c.as_object_mut().map(|o| o.insert("name".to_string(), json!(new_title)));
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

/// Write a memorph session into Cursor's state.vscdb.
///
/// This creates a new Composer session with bubbles mapped from memorph messages.
pub fn write_session(session: &MemorphSession, target_dir: &Path) -> Result<String> {
    let conn = open_global_db()?;

    // Generate a new composer ID and workspace ID
    let composer_id = Uuid::new_v4().to_string();
    let workspace_id = Uuid::new_v4().to_string().replace("-", "");
    let workspace_path = target_dir.to_string_lossy().to_string();

    // Pre-generate all bubble metadata so we can build composer indexes
    struct BubbleMeta {
        id: String,
        bubble_type: i32,
        text: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    }

    let bubbles: Vec<BubbleMeta> = session
        .messages
        .iter()
        .map(|msg| {
            let bubble_type = match msg.role {
                crate::model::MemorphRole::User => 1,
                crate::model::MemorphRole::Assistant => 2,
                _ => 2,
            };
            let text = msg
                .content
                .iter()
                .filter_map(|block| match block {
                    crate::model::ContentBlock::Text { text } => Some(text.as_str()),
                    crate::model::ContentBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
                    crate::model::ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            BubbleMeta {
                id: Uuid::new_v4().to_string(),
                bubble_type,
                text,
                timestamp: msg.timestamp,
            }
        })
        .collect();

    // Build fullConversationHeadersOnly index
    let headers: Vec<serde_json::Value> = bubbles
        .iter()
        .map(|b| {
            json!({
                "bubbleId": b.id,
                "type": b.bubble_type
            })
        })
        .collect();

    let title = session
        .session
        .title
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or("Imported from memorph");

    let first_active = session
        .messages
        .first()
        .map(|m| m.timestamp.timestamp_millis())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

    let last_active = session
        .messages
        .last()
        .map(|m| m.timestamp.timestamp_millis())
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
        "context": {
            "composers": [],
            "selectedCommits": [],
            "selectedPullRequests": [],
            "selectedImages": [],
            "folderSelections": [],
            "fileSelections": [],
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
        },
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

    // Insert composer metadata
    let composer_key = format!("composerData:{}", composer_id);
    conn.execute(
        "INSERT OR REPLACE INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
        (&composer_key, composer_data.to_string().as_bytes()),
    )
    .context("Failed to insert composer data")?;

    // Insert bubbles
    for (idx, b) in bubbles.iter().enumerate() {
        let request_id = if b.bubble_type == 1 {
            Uuid::new_v4().to_string()
        } else {
            String::new()
        };
        let bubble_data = build_bubble_json(
            b.bubble_type,
            &b.id,
            &b.text,
            &b.timestamp.to_rfc3339(),
            &request_id,
        );

        let bubble_key = format!("bubbleId:{}:{}", composer_id, b.id);
        conn.execute(
            "INSERT OR REPLACE INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            (&bubble_key, bubble_data.to_string().as_bytes()),
        )
        .with_context(|| format!("Failed to insert bubble {}", idx))?;
    }

    // Update composer.composerHeaders index
    let _ = upsert_composer_index(
        &conn,
        &composer_id,
        title,
        "",
        &workspace_id,
        &workspace_path,
        first_active,
        last_active,
    );

    // Best-effort WAL checkpoint
    let _ = conn.execute("PRAGMA wal_checkpoint(PASSIVE)", []);

    println!(
        "Wrote Cursor session {} with {} messages",
        composer_id,
        session.messages.len()
    );

    Ok(composer_id)
}

