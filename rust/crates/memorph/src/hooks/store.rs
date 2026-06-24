//! Hook event and runtime session persistence.
//!
//! Hook storage is intentionally separate from provider-native session storage
//! and from `session_state.json`. It records runtime observations and support
//! diagnostics under `~/.memorph/hooks/`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::{OnceLock, RwLock};

use crate::hooks::model::{HookEvent, RuntimeSession};
use crate::hooks::protocol::HookRuntimeEndpoint;
use crate::storage::atomic_write;

const HOOKS_DIR: &str = "hooks";
const EVENTS_FILE: &str = "events.jsonl";
const ERRORS_FILE: &str = "errors.jsonl";
const RUNTIME_SESSIONS_FILE: &str = "runtime_sessions.json";
const SERVER_RUNTIME_FILE: &str = "server_runtime.json";

#[cfg(test)]
static TEST_STORE_ROOT: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookStorePaths {
    pub root: PathBuf,
    pub events: PathBuf,
    pub errors: PathBuf,
    pub runtime_sessions: PathBuf,
    pub server_runtime: PathBuf,
}

impl HookStorePaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            events: root.join(EVENTS_FILE),
            errors: root.join(ERRORS_FILE),
            runtime_sessions: root.join(RUNTIME_SESSIONS_FILE),
            server_runtime: root.join(SERVER_RUNTIME_FILE),
            root,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RuntimeSessionStore {
    #[serde(default = "current_version")]
    pub version: u32,
    #[serde(default)]
    pub sessions: Vec<RuntimeSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookErrorRecord {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub scope: String,
    pub message: String,
}

fn current_version() -> u32 {
    1
}

pub fn hook_store_paths() -> Result<HookStorePaths> {
    #[cfg(test)]
    if let Some(root) = test_store_root() {
        return Ok(HookStorePaths::new(root));
    }

    Ok(HookStorePaths::new(
        crate::config::memorph_dir()?.join(HOOKS_DIR),
    ))
}

#[cfg(test)]
pub(crate) fn set_test_store_root(root: PathBuf) {
    let lock = TEST_STORE_ROOT.get_or_init(|| RwLock::new(None));
    *lock.write().unwrap() = Some(root);
}

#[cfg(test)]
fn test_store_root() -> Option<PathBuf> {
    TEST_STORE_ROOT
        .get_or_init(|| RwLock::new(None))
        .read()
        .unwrap()
        .clone()
}

pub fn ensure_store_dir(paths: &HookStorePaths) -> Result<()> {
    fs::create_dir_all(&paths.root).with_context(|| {
        format!(
            "Failed to create hook store directory: {}",
            paths.root.display()
        )
    })
}

pub fn append_event(event: &HookEvent) -> Result<()> {
    let paths = hook_store_paths()?;
    append_event_to_path(&paths.events, event)
}

pub fn append_event_to_path(path: &Path, event: &HookEvent) -> Result<()> {
    append_json_line(path, event)
}

pub fn load_recent_events(limit: usize) -> Result<Vec<HookEvent>> {
    let paths = hook_store_paths()?;
    load_recent_events_from_path(&paths.events, limit)
}

pub fn load_recent_events_from_path(path: &Path, limit: usize) -> Result<Vec<HookEvent>> {
    if limit == 0 || !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to open hook events file: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for line in reader.lines() {
        let line =
            line.with_context(|| format!("Failed to read hook events file: {}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        let event: HookEvent = serde_json::from_str(&line).with_context(|| {
            format!(
                "Failed to parse hook event JSONL line in {}",
                path.display()
            )
        })?;
        events.push(event);
    }
    if events.len() > limit {
        Ok(events.split_off(events.len() - limit))
    } else {
        Ok(events)
    }
}

pub fn append_error(scope: impl Into<String>, message: impl Into<String>) -> Result<()> {
    let paths = hook_store_paths()?;
    append_error_to_path(&paths.errors, scope, message)
}

pub fn load_recent_errors(limit: usize) -> Result<Vec<HookErrorRecord>> {
    let paths = hook_store_paths()?;
    load_recent_errors_from_path(&paths.errors, limit)
}

pub fn load_recent_errors_from_path(path: &Path, limit: usize) -> Result<Vec<HookErrorRecord>> {
    if limit == 0 || !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to open hook errors file: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut errors = Vec::new();
    for line in reader.lines() {
        let line =
            line.with_context(|| format!("Failed to read hook errors file: {}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        let record: HookErrorRecord = serde_json::from_str(&line).with_context(|| {
            format!(
                "Failed to parse hook error JSONL line in {}",
                path.display()
            )
        })?;
        errors.push(record);
    }
    if errors.len() > limit {
        Ok(errors.split_off(errors.len() - limit))
    } else {
        Ok(errors)
    }
}

pub fn append_error_to_path(
    path: &Path,
    scope: impl Into<String>,
    message: impl Into<String>,
) -> Result<()> {
    let record = HookErrorRecord {
        timestamp: chrono::Utc::now(),
        scope: scope.into(),
        message: message.into(),
    };
    append_json_line(path, &record)
}

pub fn save_runtime_sessions(store: &RuntimeSessionStore) -> Result<()> {
    let paths = hook_store_paths()?;
    save_runtime_sessions_to_path(&paths.runtime_sessions, store)
}

pub fn save_runtime_sessions_to_path(path: &Path, store: &RuntimeSessionStore) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("Path has no parent directory: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "Failed to create hook store directory: {}",
            parent.display()
        )
    })?;
    let raw = serde_json::to_string_pretty(store)?;
    atomic_write::write_string_atomic(path, &raw)
}

pub fn load_runtime_sessions() -> Result<RuntimeSessionStore> {
    let paths = hook_store_paths()?;
    load_runtime_sessions_from_path(&paths.runtime_sessions)
}

pub fn load_runtime_sessions_from_path(path: &Path) -> Result<RuntimeSessionStore> {
    if !path.exists() {
        return Ok(RuntimeSessionStore::default());
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("Failed to read runtime sessions file: {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse runtime sessions file: {}", path.display()))
}

pub fn save_server_runtime(endpoint: &HookRuntimeEndpoint) -> Result<()> {
    let paths = hook_store_paths()?;
    save_server_runtime_to_path(&paths.server_runtime, endpoint)
}

pub fn save_server_runtime_to_path(path: &Path, endpoint: &HookRuntimeEndpoint) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("Path has no parent directory: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "Failed to create hook store directory: {}",
            parent.display()
        )
    })?;
    let raw = serde_json::to_string_pretty(endpoint)?;
    atomic_write::write_string_atomic(path, &raw)
}

pub fn load_server_runtime() -> Result<Option<HookRuntimeEndpoint>> {
    let paths = hook_store_paths()?;
    load_server_runtime_from_path(&paths.server_runtime)
}

pub fn load_server_runtime_from_path(path: &Path) -> Result<Option<HookRuntimeEndpoint>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).with_context(|| {
        format!(
            "Failed to read hook server runtime file: {}",
            path.display()
        )
    })?;
    serde_json::from_str(&raw).map(Some).with_context(|| {
        format!(
            "Failed to parse hook server runtime file: {}",
            path.display()
        )
    })
}

fn append_json_line<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("Path has no parent directory: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "Failed to create hook store directory: {}",
            parent.display()
        )
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| {
            format!(
                "Failed to open hook JSONL file for append: {}",
                path.display()
            )
        })?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")
        .with_context(|| format!("Failed to write hook JSONL line: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::{
        HookEvent, HookEventType, RuntimeSessionId, RuntimeSessionStatus,
    };
    use serde_json::Value;

    #[test]
    fn appends_and_loads_recent_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        for idx in 0..3 {
            let mut event = HookEvent::new("generic", HookEventType::Heartbeat, Value::Null);
            event.event_id = format!("event-{idx}");
            append_event_to_path(&path, &event).unwrap();
        }
        let events = load_recent_events_from_path(&path, 2).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_id, "event-1");
        assert_eq!(events[1].event_id, "event-2");
    }

    #[test]
    fn appends_and_loads_recent_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("errors.jsonl");
        for idx in 0..3 {
            append_error_to_path(&path, "test", format!("error-{idx}")).unwrap();
        }

        let errors = load_recent_errors_from_path(&path, 2).unwrap();
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].message, "error-1");
        assert_eq!(errors[1].message, "error-2");
    }

    #[test]
    fn saves_and_loads_runtime_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime_sessions.json");
        let now = chrono::Utc::now();
        let store = RuntimeSessionStore {
            version: 1,
            sessions: vec![RuntimeSession {
                runtime_id: RuntimeSessionId::new("runtime-1"),
                provider: "generic".to_string(),
                provider_session_id: Some("session-1".to_string()),
                run_id: None,
                cwd: None,
                pid: None,
                parent_pid: None,
                pid_start_time: None,
                tty: None,
                terminal_vars: Default::default(),
                process_ancestry: Vec::new(),
                correlation: None,
                model: None,
                session_title: None,
                transcript_path: None,
                workspace_roots: Vec::new(),
                last_user_prompt: None,
                last_assistant_message: None,
                last_tool_result: None,
                last_error: None,
                stop_reason: None,
                compact_count: 0,
                tool_call_count: 0,
                failed_tool_count: 0,
                permission_request_count: 0,
                question_count: 0,
                status: RuntimeSessionStatus::Running,
                current_tool: None,
                pending_permission: None,
                pending_question: None,
                recent_activity: Vec::new(),
                subagents: Default::default(),
                last_event_at: now,
                updated_at: now,
            }],
        };
        save_runtime_sessions_to_path(&path, &store).unwrap();
        let loaded = load_runtime_sessions_from_path(&path).unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.sessions[0].runtime_id.0, "runtime-1");
    }

    #[test]
    fn loads_runtime_sessions_written_before_optional_identity_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime_sessions.json");
        let now = chrono::Utc::now().to_rfc3339();
        std::fs::write(
            &path,
            serde_json::json!({
                "version": 1,
                "sessions": [{
                    "runtime_id": "runtime-old",
                    "provider": "generic",
                    "status": "running",
                    "last_event_at": now,
                    "updated_at": now
                }]
            })
            .to_string(),
        )
        .unwrap();

        let loaded = load_runtime_sessions_from_path(&path).unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.sessions[0].runtime_id.0, "runtime-old");
        assert_eq!(loaded.sessions[0].pid_start_time, None);
        assert_eq!(loaded.sessions[0].correlation, None);
        assert!(loaded.sessions[0].recent_activity.is_empty());
        assert!(loaded.sessions[0].subagents.is_empty());
        assert_eq!(loaded.sessions[0].model, None);
        assert_eq!(loaded.sessions[0].session_title, None);
        assert!(loaded.sessions[0].workspace_roots.is_empty());
        assert_eq!(loaded.sessions[0].tool_call_count, 0);
    }

}
