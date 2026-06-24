use crate::provider::ProviderSessionSummary;
use crate::providers::cursor::db::{ComposerData, global_state_db_path, list_composers};
use anyhow::Result;
use std::path::Path;

/// Scan Cursor Composer sessions and filter by workspace path.
pub fn scan_sessions(workspace: Option<&Path>) -> Result<Vec<ProviderSessionSummary>> {
    if !global_state_db_path()?.exists() {
        return Ok(Vec::new());
    }

    let composers = list_composers()?;
    let mut sessions = Vec::new();

    for composer in composers {
        // Filter by workspace if provided
        if let Some(ws) = workspace {
            if let Some(ref wi) = composer.workspace_identifier {
                let composer_path = Path::new(&wi.uri.fs_path);
                if !paths_match(ws, composer_path) {
                    continue;
                }
            } else {
                continue;
            }
        }

        sessions.push(composer_to_session_meta(&composer));
    }

    Ok(sessions)
}

fn composer_to_session_meta(composer: &ComposerData) -> ProviderSessionSummary {
    let last_active = composer.created_at;

    ProviderSessionSummary {
        session_id: composer.composer_id.clone(),
        title: composer
            .text
            .clone()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| composer.name.clone().filter(|n| !n.trim().is_empty())),
        project_dir: composer
            .workspace_identifier
            .as_ref()
            .map(|w| w.uri.fs_path.clone()),
        last_active_at: last_active,
        source_path: Some(composer.composer_id.clone()),
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
