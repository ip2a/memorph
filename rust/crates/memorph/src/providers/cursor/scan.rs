use crate::provider::ProviderSessionSummary;
use crate::providers::cursor::db::{
    cursor_source_locator, global_state_db_path, list_session_metadata, CursorSessionMetadata,
};
use anyhow::Result;
use std::path::Path;

/// Scan current Cursor Composer sessions and filter by workspace path.
pub fn scan_sessions(workspace: Option<&Path>) -> Result<Vec<ProviderSessionSummary>> {
    if !global_state_db_path()?.exists() {
        return Ok(Vec::new());
    }

    let sessions = list_session_metadata()?;
    let mut summaries = Vec::new();

    for session in sessions {
        let project_dir = session.workspace_dir();
        if let Some(workspace) = workspace {
            let Some(project_dir) = project_dir.as_deref() else {
                continue;
            };
            if !paths_match(workspace, Path::new(project_dir)) {
                continue;
            }
        }

        let source_path = cursor_source_locator(&session.composer_id)?;
        summaries.push(session_to_summary(&session, project_dir, source_path));
    }

    Ok(summaries)
}

fn session_to_summary(
    session: &CursorSessionMetadata,
    project_dir: Option<String>,
    source_path: String,
) -> ProviderSessionSummary {
    ProviderSessionSummary {
        session_id: session.composer_id.clone(),
        title: session.title(),
        project_dir,
        created_at: session.created_at_ms(),
        last_active_at: session.last_active_at_ms(),
        source_path: Some(source_path),
    }
}

/// Check if two paths refer to the same directory.
fn paths_match(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if let (Ok(a_canon), Ok(b_canon)) = (a.canonicalize(), b.canonicalize()) {
        return a_canon == b_canon;
    }
    false
}
