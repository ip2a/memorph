use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindParams {
    pub dir: Option<String>,
    pub session: Option<String>,
    pub providers: Vec<String>,
}

pub fn find_sessions(params: &FindParams) -> Result<Vec<SessionGroup>> {
    let groups = projection::list_sessions(&SessionListParams {
        all: true,
        providers: params.providers.clone(),
        cwd: None,
        include_message_counts: true,
        limit: None,
        offset: None,
        sort: SessionListSort::Recent,
    })?;

    Ok(groups
        .into_iter()
        .filter_map(|mut group| {
            group.sessions.retain(|session| {
                let dir_match = params.dir.as_ref().is_none_or(|directory| {
                    session
                        .project_dir
                        .as_ref()
                        .is_some_and(|project_dir| project_dir.contains(directory))
                });
                let session_match = params.session.as_ref().is_none_or(|pattern| {
                    session.session_id.contains(pattern)
                        || session
                            .title
                            .as_ref()
                            .is_some_and(|title| title.contains(pattern))
                        || session
                            .native_title
                            .as_ref()
                            .is_some_and(|title| title.contains(pattern))
                });
                dir_match && session_match
            });
            (!group.sessions.is_empty()).then_some(group)
        })
        .collect())
}
