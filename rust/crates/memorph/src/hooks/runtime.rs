//! Runtime session state machine.
//!
//! The reducer turns canonical hook events into active runtime session state.
//! It is intentionally independent from API and UI code.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::hooks::identity::runtime_session_id_for_event;
use crate::hooks::model::{
    HookEvent, HookEventType, RuntimeActivity, RuntimeActivityKind, RuntimeSession,
    RuntimeSessionCorrelation, RuntimeSessionId, RuntimeSessionStatus, RuntimeSubagent,
    RuntimeSubagentStatus,
};

const RECENT_ACTIVITY_LIMIT: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RuntimeState {
    #[serde(default)]
    pub sessions: BTreeMap<RuntimeSessionId, RuntimeSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEffect {
    SessionCreated { runtime_id: RuntimeSessionId },
    SessionUpdated { runtime_id: RuntimeSessionId },
    PermissionQueued { runtime_id: RuntimeSessionId },
    QuestionQueued { runtime_id: RuntimeSessionId },
    SessionCompleted { runtime_id: RuntimeSessionId },
    SessionFailed { runtime_id: RuntimeSessionId },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RuntimeCleanupReport {
    pub idle: usize,
    pub orphaned: usize,
}

impl RuntimeState {
    pub fn apply_event(&mut self, event: &HookEvent) -> Vec<RuntimeEffect> {
        let runtime_id = runtime_session_id_for_event(event);
        let now = Utc::now();
        let mut effects = Vec::new();
        let session = self.sessions.entry(runtime_id.clone()).or_insert_with(|| {
            effects.push(RuntimeEffect::SessionCreated {
                runtime_id: runtime_id.clone(),
            });
            RuntimeSession {
                runtime_id: runtime_id.clone(),
                provider: event.provider.clone(),
                provider_session_id: event.provider_session_id.clone(),
                run_id: event.run_id.clone(),
                cwd: event.cwd.clone(),
                pid: event.pid,
                parent_pid: event.parent_pid,
                pid_start_time: event.pid_start_time.clone(),
                tty: event.tty.clone(),
                terminal_vars: event.terminal_vars.clone(),
                process_ancestry: event.process_ancestry.clone(),
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
                status: RuntimeSessionStatus::Idle,
                current_tool: None,
                pending_permission: None,
                pending_question: None,
                recent_activity: Vec::new(),
                subagents: BTreeMap::new(),
                last_event_at: event.timestamp,
                updated_at: now,
            }
        });

        session.provider = event.provider.clone();
        if event.provider_session_id.is_some() {
            session.provider_session_id = event.provider_session_id.clone();
        }
        if event.run_id.is_some() {
            session.run_id = event.run_id.clone();
        }
        if event.cwd.is_some() {
            session.cwd = event.cwd.clone();
        }
        if event.pid.is_some() {
            session.pid = event.pid;
        }
        if event.parent_pid.is_some() {
            session.parent_pid = event.parent_pid;
        }
        if event.pid_start_time.is_some() {
            session.pid_start_time = event.pid_start_time.clone();
        }
        if event.tty.is_some() {
            session.tty = event.tty.clone();
        }
        if !event.terminal_vars.is_empty() {
            session.terminal_vars = event.terminal_vars.clone();
        }
        if !event.process_ancestry.is_empty() {
            session.process_ancestry = event.process_ancestry.clone();
        }
        session.last_event_at = event.timestamp;
        session.updated_at = now;
        append_recent_activity(session, event);
        update_semantic_snapshot(session, event);
        apply_subagent_event(session, event);

        match event.event_type {
            HookEventType::SessionStarted
            | HookEventType::MessageCreated
            | HookEventType::Heartbeat => {
                if !matches!(
                    session.status,
                    RuntimeSessionStatus::WaitingPermission | RuntimeSessionStatus::WaitingUser
                ) {
                    session.status = RuntimeSessionStatus::Running;
                }
                effects.push(RuntimeEffect::SessionUpdated { runtime_id });
            }
            HookEventType::ToolStarted => {
                session.status = RuntimeSessionStatus::Running;
                session.current_tool = event.tool.clone();
                session.tool_call_count = session.tool_call_count.saturating_add(1);
                effects.push(RuntimeEffect::SessionUpdated { runtime_id });
            }
            HookEventType::ToolFinished => {
                session.status = RuntimeSessionStatus::Running;
                session.current_tool = None;
                effects.push(RuntimeEffect::SessionUpdated { runtime_id });
            }
            HookEventType::PermissionRequested => {
                session.status = RuntimeSessionStatus::WaitingPermission;
                session.pending_permission = event.permission.clone();
                session.permission_request_count =
                    session.permission_request_count.saturating_add(1);
                if session.current_tool.is_none() {
                    session.current_tool = event.tool.clone();
                }
                effects.push(RuntimeEffect::PermissionQueued { runtime_id });
            }
            HookEventType::QuestionRequested => {
                session.status = RuntimeSessionStatus::WaitingUser;
                session.pending_question = event.question.clone();
                session.question_count = session.question_count.saturating_add(1);
                effects.push(RuntimeEffect::QuestionQueued { runtime_id });
            }
            HookEventType::SessionCompleted => {
                session.status = RuntimeSessionStatus::Completed;
                session.current_tool = None;
                session.pending_permission = None;
                session.pending_question = None;
                effects.push(RuntimeEffect::SessionCompleted { runtime_id });
            }
            HookEventType::SessionFailed => {
                session.status = RuntimeSessionStatus::Failed;
                session.current_tool = None;
                effects.push(RuntimeEffect::SessionFailed { runtime_id });
            }
            HookEventType::Unknown => {
                if matches!(session.status, RuntimeSessionStatus::Idle) {
                    session.status = RuntimeSessionStatus::Running;
                }
                effects.push(RuntimeEffect::SessionUpdated { runtime_id });
            }
        }

        effects
    }

    pub fn attach_correlation(
        &mut self,
        runtime_id: &RuntimeSessionId,
        correlation: RuntimeSessionCorrelation,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(runtime_id) else {
            return false;
        };
        if session.correlation.as_ref() == Some(&correlation) {
            return false;
        }
        session.correlation = Some(correlation);
        session.updated_at = Utc::now();
        true
    }

    pub fn cleanup_stale_sessions(
        &mut self,
        now: DateTime<Utc>,
        idle_after: Duration,
        orphan_after: Duration,
        is_pid_alive: impl Fn(u32, Option<&str>) -> bool,
    ) -> RuntimeCleanupReport {
        let mut report = RuntimeCleanupReport::default();
        for session in self.sessions.values_mut() {
            if matches!(
                session.status,
                RuntimeSessionStatus::Completed | RuntimeSessionStatus::Failed
            ) {
                continue;
            }
            if matches!(
                session.status,
                RuntimeSessionStatus::WaitingPermission | RuntimeSessionStatus::WaitingUser
            ) {
                continue;
            }

            let age = now.signed_duration_since(session.last_event_at);
            if let Some(pid) = session.pid {
                if age >= orphan_after && !is_pid_alive(pid, session.pid_start_time.as_deref()) {
                    session.status = RuntimeSessionStatus::Orphaned;
                    session.updated_at = now;
                    report.orphaned += 1;
                    continue;
                }
            }
            if age >= idle_after && matches!(session.status, RuntimeSessionStatus::Running) {
                session.status = RuntimeSessionStatus::Idle;
                session.updated_at = now;
                report.idle += 1;
            }
        }
        report
    }

    pub fn active_sessions(&self) -> Vec<&RuntimeSession> {
        self.sessions
            .values()
            .filter(|session| {
                !matches!(
                    session.status,
                    RuntimeSessionStatus::Completed | RuntimeSessionStatus::Failed
                )
            })
            .collect()
    }
}

fn apply_subagent_event(session: &mut RuntimeSession, event: &HookEvent) {
    let Some(agent_id) = subagent_id(event) else {
        return;
    };
    let provider_event_name = provider_event_name(event);
    let normalized = provider_event_name
        .as_deref()
        .map(normalize_activity_name)
        .unwrap_or_else(|| canonical_subagent_event_name(&event.event_type));
    let agent_type = subagent_type(event).unwrap_or_else(|| "Agent".to_string());
    let subagent = session
        .subagents
        .entry(agent_id.clone())
        .or_insert_with(|| RuntimeSubagent {
            id: agent_id.clone(),
            agent_type: agent_type.clone(),
            status: RuntimeSubagentStatus::Processing,
            current_tool: None,
            last_event_name: None,
            started_at: event.timestamp,
            last_event_at: event.timestamp,
            completed_at: None,
        });

    if subagent.agent_type == "Agent" && agent_type != "Agent" {
        subagent.agent_type = agent_type;
    }
    subagent.last_event_name = provider_event_name;
    subagent.last_event_at = event.timestamp;

    match normalized.as_str() {
        "subagentstart" | "sessionstart" => {
            subagent.status = RuntimeSubagentStatus::Running;
            subagent.current_tool = None;
            subagent.completed_at = None;
        }
        "userpromptsubmit" | "userpromptsubmitted" | "taskresume" => {
            subagent.status = RuntimeSubagentStatus::Processing;
            subagent.current_tool = None;
            subagent.completed_at = None;
        }
        "pretooluse" | "toolstarted" | "toolstart" | "toolcall" => {
            subagent.status = RuntimeSubagentStatus::Running;
            subagent.current_tool = event.tool.clone();
            subagent.completed_at = None;
        }
        "posttooluse" | "posttoolusefailure" | "toolfinished" | "toolfinish" | "toolend" => {
            subagent.status = RuntimeSubagentStatus::Processing;
            subagent.current_tool = None;
            subagent.completed_at = None;
        }
        "subagentstop" | "stop" | "sessionend" => {
            subagent.status = RuntimeSubagentStatus::Completed;
            subagent.current_tool = None;
            subagent.completed_at = Some(event.timestamp);
        }
        "sessionfailed" | "sessionfail" | "failed" | "error" | "taskcancel" | "erroroccurred" => {
            subagent.status = RuntimeSubagentStatus::Failed;
            subagent.current_tool = None;
            subagent.completed_at = Some(event.timestamp);
        }
        _ => {}
    }

    if !matches!(
        session.status,
        RuntimeSessionStatus::WaitingPermission | RuntimeSessionStatus::WaitingUser
    ) && session.subagents.values().any(|subagent| {
        matches!(
            subagent.status,
            RuntimeSubagentStatus::Processing | RuntimeSubagentStatus::Running
        )
    }) {
        session.status = RuntimeSessionStatus::Running;
    }
}

fn update_semantic_snapshot(session: &mut RuntimeSession, event: &HookEvent) {
    let is_subagent_event = subagent_id(event).is_some();
    if !is_subagent_event {
        assign_if_some(
            &mut session.model,
            string_deep(&event.raw, &["model", "model_name", "modelName"]),
        );
        assign_if_some(
            &mut session.session_title,
            string_deep(
                &event.raw,
                &[
                    "session_title",
                    "sessionTitle",
                    "title",
                    "conversation_title",
                    "conversationTitle",
                ],
            ),
        );
        if let Some(path) = string_deep(&event.raw, &["transcript_path", "transcriptPath"]) {
            session.transcript_path = Some(path.into());
        }
        let roots = path_list_deep(
            &event.raw,
            &[
                "workspace_roots",
                "workspaceRoots",
                "workspace_folders",
                "workspaceFolders",
            ],
        );
        if !roots.is_empty() {
            session.workspace_roots = roots;
        }
    }

    let provider_event_name = provider_event_name(event);
    let provider_event = provider_event_name
        .as_deref()
        .map(normalize_activity_name)
        .unwrap_or_default();

    if event
        .message
        .as_ref()
        .and_then(|message| message.role.as_deref())
        == Some("user")
        || provider_event == "userpromptsubmit"
        || provider_event == "userpromptsubmitted"
        || provider_event == "beforesubmitprompt"
    {
        assign_if_some(
            &mut session.last_user_prompt,
            event
                .message
                .as_ref()
                .map(|message| message.text.clone())
                .or_else(|| {
                    string_deep(
                        &event.raw,
                        &[
                            "prompt",
                            "last_user_message",
                            "lastUserMessage",
                            "user_message",
                            "userMessage",
                            "message",
                            "text",
                            "content",
                        ],
                    )
                }),
        );
    }

    if event
        .message
        .as_ref()
        .and_then(|message| message.role.as_deref())
        == Some("assistant")
        || provider_event == "afteragentresponse"
        || provider_event == "stop"
    {
        assign_if_some(
            &mut session.last_assistant_message,
            event
                .message
                .as_ref()
                .map(|message| message.text.clone())
                .or_else(|| {
                    string_deep(
                        &event.raw,
                        &[
                            "last_assistant_message",
                            "lastAssistantMessage",
                            "assistant_message",
                            "assistantMessage",
                            "summary",
                            "message",
                            "text",
                            "content",
                        ],
                    )
                }),
        );
    }

    if matches!(event.event_type, HookEventType::ToolFinished) {
        assign_if_some(
            &mut session.last_tool_result,
            string_deep(
                &event.raw,
                &[
                    "tool_result",
                    "toolResult",
                    "result",
                    "output",
                    "stdout",
                    "observation",
                ],
            ),
        );
        if provider_event == "posttoolusefailure" || provider_event == "post_tool_use_failure" {
            session.failed_tool_count = session.failed_tool_count.saturating_add(1);
        }
    }

    let error = string_deep(
        &event.raw,
        &[
            "error",
            "error_message",
            "errorMessage",
            "tool_error",
            "toolError",
            "failure",
            "stderr",
        ],
    );
    if error.is_some() {
        assign_if_some(&mut session.last_error, error);
    }

    if matches!(
        provider_event.as_str(),
        "precompact" | "postcompact" | "sessionbeforecompact" | "sessioncompact"
    ) {
        if matches!(
            provider_event.as_str(),
            "precompact" | "sessionbeforecompact"
        ) {
            session.compact_count = session.compact_count.saturating_add(1);
        }
        assign_if_some(
            &mut session.last_tool_result,
            string_deep(
                &event.raw,
                &[
                    "compact_summary",
                    "compactSummary",
                    "summary",
                    "message",
                    "text",
                ],
            ),
        );
    }

    if matches!(
        event.event_type,
        HookEventType::SessionCompleted | HookEventType::SessionFailed
    ) || provider_event == "stop"
    {
        assign_if_some(
            &mut session.stop_reason,
            string_deep(
                &event.raw,
                &[
                    "stop_reason",
                    "stopReason",
                    "reason",
                    "status",
                    "finish_reason",
                ],
            ),
        );
    }

    if matches!(event.event_type, HookEventType::SessionFailed) && session.last_error.is_none() {
        assign_if_some(
            &mut session.last_error,
            string_deep(&event.raw, &["message", "summary", "reason"]),
        );
    }
}

fn assign_if_some(target: &mut Option<String>, value: Option<String>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        *target = Some(preview_text(&value));
    }
}

fn append_recent_activity(session: &mut RuntimeSession, event: &HookEvent) {
    session.recent_activity.push(activity_for_event(event));
    let overflow = session
        .recent_activity
        .len()
        .saturating_sub(RECENT_ACTIVITY_LIMIT);
    if overflow > 0 {
        session.recent_activity.drain(0..overflow);
    }
}

fn activity_for_event(event: &HookEvent) -> RuntimeActivity {
    let provider_event_name = provider_event_name(event);
    let kind = activity_kind(event, provider_event_name.as_deref());
    let tool_name = event.tool.as_ref().map(|tool| tool.name.clone());
    let message_preview = activity_preview(event);
    let label = activity_label(
        event,
        &kind,
        provider_event_name.as_deref(),
        &message_preview,
    );

    RuntimeActivity {
        id: event.event_id.clone(),
        kind,
        event_type: event.event_type.clone(),
        provider_event_name,
        label,
        tool_name,
        message_preview,
        timestamp: event.timestamp,
    }
}

fn activity_kind(event: &HookEvent, provider_event_name: Option<&str>) -> RuntimeActivityKind {
    if let Some(name) = provider_event_name {
        match normalize_activity_name(name).as_str() {
            "userpromptsubmit" | "userpromptsubmitted" | "taskresume" => {
                return RuntimeActivityKind::UserPromptSubmitted;
            }
            "subagentstart" => return RuntimeActivityKind::SubagentStarted,
            "subagentstop" => return RuntimeActivityKind::SubagentStopped,
            "precompact" | "postcompact" | "sessionbeforecompact" | "sessioncompact" => {
                return RuntimeActivityKind::Compaction;
            }
            "notification" => return RuntimeActivityKind::Notification,
            _ => {}
        }
    }

    match event.event_type {
        HookEventType::SessionStarted => RuntimeActivityKind::SessionStarted,
        HookEventType::MessageCreated => RuntimeActivityKind::MessageCreated,
        HookEventType::ToolStarted => RuntimeActivityKind::ToolStarted,
        HookEventType::ToolFinished => RuntimeActivityKind::ToolFinished,
        HookEventType::PermissionRequested => RuntimeActivityKind::PermissionRequested,
        HookEventType::QuestionRequested => RuntimeActivityKind::QuestionRequested,
        HookEventType::SessionCompleted => RuntimeActivityKind::SessionCompleted,
        HookEventType::SessionFailed => RuntimeActivityKind::SessionFailed,
        HookEventType::Heartbeat => RuntimeActivityKind::Heartbeat,
        HookEventType::Unknown => RuntimeActivityKind::ProviderEvent,
    }
}

fn activity_label(
    event: &HookEvent,
    kind: &RuntimeActivityKind,
    provider_event_name: Option<&str>,
    message_preview: &Option<String>,
) -> String {
    match kind {
        RuntimeActivityKind::SessionStarted => "Session started".to_string(),
        RuntimeActivityKind::UserPromptSubmitted => message_preview
            .as_ref()
            .map(|preview| format!("User prompt: {preview}"))
            .unwrap_or_else(|| "User prompt submitted".to_string()),
        RuntimeActivityKind::MessageCreated => message_preview
            .as_ref()
            .cloned()
            .unwrap_or_else(|| "Message created".to_string()),
        RuntimeActivityKind::ToolStarted => event
            .tool
            .as_ref()
            .map(|tool| format!("Tool started: {}", tool.name))
            .unwrap_or_else(|| "Tool started".to_string()),
        RuntimeActivityKind::ToolFinished => event
            .tool
            .as_ref()
            .map(|tool| format!("Tool finished: {}", tool.name))
            .unwrap_or_else(|| "Tool finished".to_string()),
        RuntimeActivityKind::PermissionRequested => event
            .permission
            .as_ref()
            .and_then(|permission| permission.tool.as_ref())
            .map(|tool| format!("Permission requested: {}", tool.name))
            .or_else(|| {
                event
                    .tool
                    .as_ref()
                    .map(|tool| format!("Permission requested: {}", tool.name))
            })
            .unwrap_or_else(|| "Permission requested".to_string()),
        RuntimeActivityKind::QuestionRequested => message_preview
            .as_ref()
            .map(|preview| format!("Question: {preview}"))
            .unwrap_or_else(|| "Question requested".to_string()),
        RuntimeActivityKind::SubagentStarted => activity_subject(event)
            .map(|subject| format!("Subagent started: {subject}"))
            .unwrap_or_else(|| "Subagent started".to_string()),
        RuntimeActivityKind::SubagentStopped => activity_subject(event)
            .map(|subject| format!("Subagent stopped: {subject}"))
            .unwrap_or_else(|| "Subagent stopped".to_string()),
        RuntimeActivityKind::Compaction => provider_event_name
            .map(|name| format!("Context compaction: {name}"))
            .unwrap_or_else(|| "Context compaction".to_string()),
        RuntimeActivityKind::Notification => message_preview
            .as_ref()
            .map(|preview| format!("Notification: {preview}"))
            .unwrap_or_else(|| "Notification".to_string()),
        RuntimeActivityKind::SessionCompleted => "Session completed".to_string(),
        RuntimeActivityKind::SessionFailed => "Session failed".to_string(),
        RuntimeActivityKind::Heartbeat => provider_event_name
            .map(|name| format!("Heartbeat: {name}"))
            .unwrap_or_else(|| "Heartbeat".to_string()),
        RuntimeActivityKind::ProviderEvent => provider_event_name
            .map(|name| format!("Provider event: {name}"))
            .unwrap_or_else(|| "Provider event".to_string()),
    }
}

fn provider_event_name(event: &HookEvent) -> Option<String> {
    string_at(
        &event.raw,
        &[
            "hook_event_name",
            "hookEventName",
            "event_name",
            "eventName",
            "event",
            "type",
        ],
    )
}

fn activity_preview(event: &HookEvent) -> Option<String> {
    if let Some(message) = &event.message {
        return Some(preview_text(&message.text));
    }
    if let Some(question) = &event.question {
        return Some(preview_text(&question.prompt));
    }
    if let Some(prompt) = event
        .permission
        .as_ref()
        .and_then(|permission| permission.prompt.as_deref())
    {
        return Some(preview_text(prompt));
    }
    string_at(
        &event.raw,
        &[
            "prompt",
            "message",
            "text",
            "content",
            "notification",
            "summary",
            "description",
        ],
    )
    .map(|value| preview_text(&value))
}

fn activity_subject(event: &HookEvent) -> Option<String> {
    string_at(
        &event.raw,
        &[
            "agent_name",
            "agentName",
            "subagent_name",
            "subagentName",
            "agent_id",
            "agentId",
            "subagent_id",
            "subagentId",
            "description",
        ],
    )
    .map(|value| preview_text(&value))
}

fn subagent_id(event: &HookEvent) -> Option<String> {
    string_deep(
        &event.raw,
        &[
            "agent_id",
            "agentId",
            "subagent_id",
            "subagentId",
            "sub_agent_id",
            "subAgentId",
        ],
    )
}

fn subagent_type(event: &HookEvent) -> Option<String> {
    string_deep(
        &event.raw,
        &[
            "agent_type",
            "agentType",
            "subagent_type",
            "subagentType",
            "sub_agent_type",
            "subAgentType",
            "agent_name",
            "agentName",
            "subagent_name",
            "subagentName",
        ],
    )
}

fn canonical_subagent_event_name(event_type: &HookEventType) -> String {
    match event_type {
        HookEventType::SessionStarted => "sessionstart",
        HookEventType::ToolStarted => "toolstarted",
        HookEventType::ToolFinished => "toolfinished",
        HookEventType::PermissionRequested => "permissionrequested",
        HookEventType::QuestionRequested => "questionrequested",
        HookEventType::SessionCompleted => "sessionend",
        HookEventType::SessionFailed => "sessionfailed",
        HookEventType::MessageCreated => "messagecreated",
        HookEventType::Heartbeat => "heartbeat",
        HookEventType::Unknown => "unknown",
    }
    .to_string()
}

fn string_at(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|raw| match raw {
            serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    })
}

fn string_deep(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    string_at(value, keys).or_else(|| {
        ["payload", "data", "input", "params", "message", "event"]
            .iter()
            .find_map(|container| {
                value
                    .get(*container)
                    .and_then(|nested| string_at(nested, keys))
            })
    })
}

fn path_list_deep(value: &serde_json::Value, keys: &[&str]) -> Vec<PathBuf> {
    for key in keys {
        if let Some(paths) = value.get(*key).and_then(path_list_from_value) {
            return paths;
        }
    }
    for container in ["payload", "data", "input", "params"] {
        if let Some(nested) = value.get(container) {
            for key in keys {
                if let Some(paths) = nested.get(*key).and_then(path_list_from_value) {
                    return paths;
                }
            }
        }
    }
    Vec::new()
}

fn path_list_from_value(value: &serde_json::Value) -> Option<Vec<PathBuf>> {
    match value {
        serde_json::Value::String(path) if !path.trim().is_empty() => {
            Some(vec![PathBuf::from(path.trim())])
        }
        serde_json::Value::Array(items) => {
            let paths = items
                .iter()
                .filter_map(|item| match item {
                    serde_json::Value::String(path) if !path.trim().is_empty() => {
                        Some(PathBuf::from(path.trim()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            if paths.is_empty() {
                None
            } else {
                Some(paths)
            }
        }
        _ => None,
    }
}

fn preview_text(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_CHARS: usize = 160;
    if compact.chars().count() <= MAX_CHARS {
        return compact;
    }
    let mut preview = compact.chars().take(MAX_CHARS).collect::<String>();
    preview.push('…');
    preview
}

fn normalize_activity_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::{
        HookEvent, HookEventType, HookToolCall, PermissionRequest, QuestionRequest,
    };
    use serde_json::{json, Value};

    fn event(event_type: HookEventType) -> HookEvent {
        let mut event = HookEvent::new("generic", event_type, Value::Null);
        event.provider_session_id = Some("session-1".to_string());
        event
    }

    #[test]
    fn reduces_tool_lifecycle() {
        let mut state = RuntimeState::default();
        let mut started = event(HookEventType::ToolStarted);
        started.tool = Some(HookToolCall {
            id: Some("tool-1".to_string()),
            name: "Bash".to_string(),
            input: json!({"command": "cargo test"}),
        });
        state.apply_event(&started);
        let session = state.sessions.values().next().unwrap();
        assert_eq!(session.status, RuntimeSessionStatus::Running);
        assert_eq!(session.current_tool.as_ref().unwrap().name, "Bash");

        let finished = event(HookEventType::ToolFinished);
        state.apply_event(&finished);
        let session = state.sessions.values().next().unwrap();
        assert_eq!(session.status, RuntimeSessionStatus::Running);
        assert!(session.current_tool.is_none());
    }

    #[test]
    fn queues_permission_and_clears_on_completion() {
        let mut state = RuntimeState::default();
        let mut permission = event(HookEventType::PermissionRequested);
        permission.permission = Some(PermissionRequest {
            request_id: Some("perm-1".to_string()),
            tool: None,
            prompt: Some("Allow?".to_string()),
        });
        let effects = state.apply_event(&permission);
        assert!(matches!(
            effects.last(),
            Some(RuntimeEffect::PermissionQueued { .. })
        ));
        let session = state.sessions.values().next().unwrap();
        assert_eq!(session.status, RuntimeSessionStatus::WaitingPermission);
        assert!(session.pending_permission.is_some());

        state.apply_event(&event(HookEventType::SessionCompleted));
        let session = state.sessions.values().next().unwrap();
        assert_eq!(session.status, RuntimeSessionStatus::Completed);
        assert!(session.pending_permission.is_none());
    }

    #[test]
    fn attaches_correlation() {
        let mut state = RuntimeState::default();
        let event = event(HookEventType::SessionStarted);
        let runtime_id = runtime_session_id_for_event(&event);
        state.apply_event(&event);
        let changed = state.attach_correlation(
            &runtime_id,
            RuntimeSessionCorrelation {
                provider: "generic".to_string(),
                session_id: "session-1".to_string(),
                title: Some("title".to_string()),
                project_dir: None,
                source_path: None,
                matched_by: Some("provider_session_id".to_string()),
            },
        );
        assert!(changed);
        assert_eq!(
            state
                .sessions
                .get(&runtime_id)
                .unwrap()
                .correlation
                .as_ref()
                .unwrap()
                .session_id,
            "session-1"
        );
    }

    #[test]
    fn cleanup_marks_idle_and_orphaned_sessions() {
        let mut state = RuntimeState::default();
        let now = Utc::now();
        let mut idle = event(HookEventType::ToolStarted);
        idle.provider_session_id = Some("idle".to_string());
        idle.timestamp = now - Duration::minutes(40);
        state.apply_event(&idle);

        let mut orphan = event(HookEventType::ToolStarted);
        orphan.provider_session_id = Some("orphan".to_string());
        orphan.pid = Some(42);
        orphan.timestamp = now - Duration::minutes(90);
        state.apply_event(&orphan);

        let report = state.cleanup_stale_sessions(
            now,
            Duration::minutes(30),
            Duration::minutes(60),
            |_, _| false,
        );
        assert_eq!(report.idle, 1);
        assert_eq!(report.orphaned, 1);
    }

    #[test]
    fn queues_question() {
        let mut state = RuntimeState::default();
        let mut question = event(HookEventType::QuestionRequested);
        question.question = Some(QuestionRequest {
            request_id: Some("q1".to_string()),
            prompt: "Which branch?".to_string(),
        });
        state.apply_event(&question);
        let session = state.sessions.values().next().unwrap();
        assert_eq!(session.status, RuntimeSessionStatus::WaitingUser);
        assert_eq!(
            session.pending_question.as_ref().unwrap().prompt,
            "Which branch?"
        );
    }

    #[test]
    fn stores_latest_terminal_environment_on_runtime_session() {
        let mut state = RuntimeState::default();
        let mut event = event(HookEventType::SessionStarted);
        event
            .terminal_vars
            .insert("TMUX_PANE".to_string(), "%9".to_string());
        event
            .terminal_vars
            .insert("ITERM_SESSION_ID".to_string(), "w0t1p0".to_string());

        state.apply_event(&event);
        let session = state.sessions.values().next().unwrap();
        assert_eq!(
            session.terminal_vars.get("TMUX_PANE").map(String::as_str),
            Some("%9")
        );
        assert_eq!(
            session
                .terminal_vars
                .get("ITERM_SESSION_ID")
                .map(String::as_str),
            Some("w0t1p0")
        );
    }

    #[test]
    fn maintains_runtime_semantic_snapshot_from_hook_payloads() {
        let mut state = RuntimeState::default();

        let mut started = event(HookEventType::SessionStarted);
        started.raw = json!({
            "hook_event_name": "SessionStart",
            "model": "claude-opus-4",
            "session_title": "Implement hooks",
            "transcript_path": "/tmp/project/transcript.jsonl",
            "workspace_roots": ["/tmp/project", "/tmp/project/crate"]
        });
        state.apply_event(&started);

        let mut prompt = event(HookEventType::MessageCreated);
        prompt.raw = json!({
            "hook_event_name": "UserPromptSubmit",
            "prompt": "add semantic session tracking"
        });
        state.apply_event(&prompt);

        let mut tool = event(HookEventType::ToolStarted);
        tool.tool = Some(HookToolCall {
            id: Some("tool-1".to_string()),
            name: "Bash".to_string(),
            input: json!({"command": "cargo test"}),
        });
        state.apply_event(&tool);

        let mut tool_done = event(HookEventType::ToolFinished);
        tool_done.raw = json!({
            "hook_event_name": "PostToolUse",
            "tool_result": "tests passed"
        });
        state.apply_event(&tool_done);

        let mut assistant = event(HookEventType::MessageCreated);
        assistant.raw = json!({
            "hook_event_name": "Stop",
            "last_assistant_message": "Done",
            "stop_reason": "end_turn"
        });
        state.apply_event(&assistant);

        let session = state.sessions.values().next().unwrap();
        assert_eq!(session.model.as_deref(), Some("claude-opus-4"));
        assert_eq!(session.session_title.as_deref(), Some("Implement hooks"));
        assert_eq!(
            session.transcript_path.as_deref(),
            Some(std::path::Path::new("/tmp/project/transcript.jsonl"))
        );
        assert_eq!(session.workspace_roots.len(), 2);
        assert_eq!(
            session.last_user_prompt.as_deref(),
            Some("add semantic session tracking")
        );
        assert_eq!(session.last_tool_result.as_deref(), Some("tests passed"));
        assert_eq!(session.last_assistant_message.as_deref(), Some("Done"));
        assert_eq!(session.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(session.tool_call_count, 1);
    }

    #[test]
    fn tracks_failed_tools_errors_compaction_and_questions() {
        let mut state = RuntimeState::default();

        let mut question = event(HookEventType::QuestionRequested);
        question.question = Some(QuestionRequest {
            request_id: Some("q1".to_string()),
            prompt: "Which branch?".to_string(),
        });
        state.apply_event(&question);

        let mut failed_tool = event(HookEventType::ToolFinished);
        failed_tool.raw = json!({
            "hook_event_name": "PostToolUseFailure",
            "tool_error": "command failed"
        });
        state.apply_event(&failed_tool);

        let mut compact = event(HookEventType::Heartbeat);
        compact.raw = json!({
            "hook_event_name": "PreCompact",
            "summary": "context too large"
        });
        state.apply_event(&compact);

        let session = state.sessions.values().next().unwrap();
        assert_eq!(session.question_count, 1);
        assert_eq!(session.failed_tool_count, 1);
        assert_eq!(session.compact_count, 1);
        assert_eq!(session.last_error.as_deref(), Some("command failed"));
        assert_eq!(
            session.last_tool_result.as_deref(),
            Some("context too large")
        );
    }

    #[test]
    fn stores_recent_activity_for_subagent_and_notification_events() {
        let mut state = RuntimeState::default();

        let mut subagent = event(HookEventType::Heartbeat);
        subagent.raw = json!({
            "hook_event_name": "SubagentStart",
            "agent_id": "agent-1"
        });
        state.apply_event(&subagent);

        let mut notification = event(HookEventType::MessageCreated);
        notification.event_id = "notification-1".to_string();
        notification.raw = json!({
            "hook_event_name": "Notification",
            "message": "Need user attention"
        });
        state.apply_event(&notification);

        let session = state.sessions.values().next().unwrap();
        assert_eq!(session.recent_activity.len(), 2);
        assert_eq!(
            session.recent_activity[0].kind,
            RuntimeActivityKind::SubagentStarted
        );
        assert_eq!(
            session.recent_activity[0].provider_event_name.as_deref(),
            Some("SubagentStart")
        );
        assert!(session.recent_activity[0]
            .label
            .contains("Subagent started"));
        assert_eq!(
            session.recent_activity[1].kind,
            RuntimeActivityKind::Notification
        );
        assert_eq!(
            session.recent_activity[1].message_preview.as_deref(),
            Some("Need user attention")
        );
    }

    #[test]
    fn tracks_subagent_lifecycle_on_runtime_session() {
        let mut state = RuntimeState::default();

        let mut started = event(HookEventType::Heartbeat);
        started.raw = json!({
            "hook_event_name": "SubagentStart",
            "agent_id": "agent-1",
            "agent_type": "Search"
        });
        state.apply_event(&started);

        let mut tool_started = event(HookEventType::ToolStarted);
        tool_started.raw = json!({
            "hook_event_name": "PreToolUse",
            "agent_id": "agent-1",
            "agent_type": "Search"
        });
        tool_started.tool = Some(HookToolCall {
            id: Some("tool-1".to_string()),
            name: "Grep".to_string(),
            input: json!({"pattern": "RuntimeSession"}),
        });
        state.apply_event(&tool_started);

        let session = state.sessions.values().next().unwrap();
        let subagent = session.subagents.get("agent-1").unwrap();
        assert_eq!(subagent.agent_type, "Search");
        assert_eq!(subagent.status, RuntimeSubagentStatus::Running);
        assert_eq!(subagent.current_tool.as_ref().unwrap().name, "Grep");

        let mut stopped = event(HookEventType::Heartbeat);
        stopped.raw = json!({
            "hook_event_name": "SubagentStop",
            "agent_id": "agent-1"
        });
        state.apply_event(&stopped);

        let session = state.sessions.values().next().unwrap();
        let subagent = session.subagents.get("agent-1").unwrap();
        assert_eq!(subagent.status, RuntimeSubagentStatus::Completed);
        assert!(subagent.current_tool.is_none());
        assert!(subagent.completed_at.is_some());
    }

    #[test]
    fn subagent_activity_does_not_override_waiting_state() {
        let mut state = RuntimeState::default();

        let mut permission = event(HookEventType::PermissionRequested);
        permission.permission = Some(PermissionRequest {
            request_id: Some("perm-1".to_string()),
            tool: None,
            prompt: Some("Allow?".to_string()),
        });
        state.apply_event(&permission);

        let mut subagent = event(HookEventType::Heartbeat);
        subagent.raw = json!({
            "hook_event_name": "SubagentStart",
            "agent_id": "agent-1"
        });
        state.apply_event(&subagent);

        let session = state.sessions.values().next().unwrap();
        assert_eq!(session.status, RuntimeSessionStatus::WaitingPermission);
        assert_eq!(
            session.subagents.get("agent-1").unwrap().status,
            RuntimeSubagentStatus::Running
        );
    }

    #[test]
    fn recent_activity_is_bounded() {
        let mut state = RuntimeState::default();

        for idx in 0..25 {
            let mut heartbeat = event(HookEventType::Heartbeat);
            heartbeat.event_id = format!("event-{idx}");
            heartbeat.raw = json!({"hook_event_name": "Heartbeat"});
            state.apply_event(&heartbeat);
        }

        let session = state.sessions.values().next().unwrap();
        assert_eq!(session.recent_activity.len(), RECENT_ACTIVITY_LIMIT);
        assert_eq!(session.recent_activity[0].id, "event-5");
        assert_eq!(session.recent_activity[19].id, "event-24");
    }
}
