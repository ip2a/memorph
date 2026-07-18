use anyhow::{anyhow, Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use walkdir::WalkDir;

const MANAGED_MARKER: &str = ".memorph-managed-skill";

#[derive(Clone, Debug, Serialize)]
pub struct SkillAgent {
    pub provider_id: String,
    pub name: String,
    pub skills_dir: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillInstallation {
    pub provider_id: String,
    pub path: PathBuf,
    pub managed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub directory: String,
    pub installations: Vec<SkillInstallation>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillsOverview {
    pub agents: Vec<SkillAgent>,
    pub skills: Vec<SkillEntry>,
}

#[derive(Clone)]
struct SkillsState {
    agents: Arc<Vec<SkillAgent>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SkillMutation {
    skill_id: String,
    provider: String,
}

#[derive(Serialize)]
struct ApiResponse<T: Serialize> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    fn success(data: T) -> Json<Self> {
        Json(Self {
            ok: true,
            data: Some(data),
            error: None,
        })
    }
}

pub fn router() -> Router {
    router_for(default_agents())
}

fn router_for(agents: Vec<SkillAgent>) -> Router {
    Router::new()
        .route("/api/v1/skills", get(list_skills))
        .route(
            "/api/v1/skills/install",
            post(install_skill).delete(uninstall_skill),
        )
        .with_state(SkillsState {
            agents: Arc::new(agents),
        })
}

fn default_agents() -> Vec<SkillAgent> {
    let home = dirs::home_dir().unwrap_or_default();
    vec![
        SkillAgent {
            provider_id: "claude".into(),
            name: "Claude Code".into(),
            skills_dir: home.join(".claude/skills"),
        },
        SkillAgent {
            provider_id: "codex".into(),
            name: "Codex".into(),
            skills_dir: home.join(".codex/skills"),
        },
        SkillAgent {
            provider_id: "gemini".into(),
            name: "Gemini CLI".into(),
            skills_dir: home.join(".gemini/skills"),
        },
        SkillAgent {
            provider_id: "opencode".into(),
            name: "OpenCode".into(),
            skills_dir: home.join(".config/opencode/skills"),
        },
        SkillAgent {
            provider_id: "hermes".into(),
            name: "Hermes".into(),
            skills_dir: home.join(".hermes/skills"),
        },
    ]
}

fn discover(agents: &[SkillAgent]) -> SkillsOverview {
    let mut skills: BTreeMap<String, SkillEntry> = BTreeMap::new();

    for agent in agents {
        let Ok(children) = fs::read_dir(&agent.skills_dir) else {
            continue;
        };
        let mut children = children.filter_map(Result::ok).collect::<Vec<_>>();
        children.sort_by_key(|entry| entry.file_name());

        for child in children {
            let path = child.path();
            if !path.is_dir() || !path.join("SKILL.md").is_file() {
                continue;
            }
            let directory = child.file_name().to_string_lossy().into_owned();
            let (name, description) = read_metadata(&path.join("SKILL.md"), &directory);
            let id = {
                let normalized = skill_id(&name);
                if normalized.is_empty() {
                    skill_id(&directory)
                } else {
                    normalized
                }
            };
            let installation = SkillInstallation {
                provider_id: agent.provider_id.clone(),
                managed: path.join(MANAGED_MARKER).is_file(),
                path,
            };
            let skill = skills.entry(id.clone()).or_insert_with(|| SkillEntry {
                id,
                name,
                description: description.clone(),
                directory,
                installations: Vec::new(),
            });
            if skill.description.is_none() {
                skill.description = description;
            }
            skill.installations.push(installation);
        }
    }

    SkillsOverview {
        agents: agents.to_vec(),
        skills: skills.into_values().collect(),
    }
}

fn read_metadata(path: &Path, directory: &str) -> (String, Option<String>) {
    let Ok(contents) = fs::read_to_string(path) else {
        return (directory.to_string(), None);
    };
    let mut name = None;
    let mut description = None;
    let mut lines = contents.lines();
    if lines.next().map(str::trim) == Some("---") {
        for line in lines {
            let line = line.trim();
            if line == "---" {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                let value = value.trim().trim_matches(['\'', '"']);
                match key.trim() {
                    "name" if !value.is_empty() => name = Some(value.to_string()),
                    "description" if !value.is_empty() => description = Some(value.to_string()),
                    _ => {}
                }
            }
        }
    }
    (name.unwrap_or_else(|| directory.to_string()), description)
}

fn skill_id(name: &str) -> String {
    let mut id = String::new();
    let mut separator = false;
    for ch in name.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            if separator && !id.is_empty() {
                id.push('-');
            }
            separator = false;
            id.push(ch);
        } else {
            separator = true;
        }
    }
    id.trim_matches('-').to_string()
}

fn validate_directory(directory: &str) -> Result<()> {
    let path = Path::new(directory);
    if directory.is_empty()
        || directory == "."
        || directory == ".."
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(anyhow!("Unsafe skill directory: {directory}"));
    }
    Ok(())
}

fn install(agents: &[SkillAgent], request: &SkillMutation) -> Result<SkillsOverview> {
    let overview = discover(agents);
    let skill = overview
        .skills
        .iter()
        .find(|skill| skill.id == request.skill_id)
        .ok_or_else(|| anyhow!("Unknown skill: {}", request.skill_id))?;
    validate_directory(&skill.directory)?;
    let source = skill
        .installations
        .first()
        .ok_or_else(|| anyhow!("Skill has no source installation"))?;
    let agent = agents
        .iter()
        .find(|agent| agent.provider_id == request.provider)
        .ok_or_else(|| anyhow!("Unknown skill provider: {}", request.provider))?;
    let destination = agent.skills_dir.join(&skill.directory);
    if destination.exists() {
        return Err(anyhow!(
            "Skill {} is already installed for {}",
            skill.name,
            agent.name
        ));
    }

    copy_skill(&source.path, &destination)?;
    if let Err(error) = fs::write(destination.join(MANAGED_MARKER), b"managed by memorph\n") {
        let _ = fs::remove_dir_all(&destination);
        return Err(error).with_context(|| format!("Failed to mark {}", destination.display()));
    }
    Ok(discover(agents))
}

fn copy_skill(source: &Path, destination: &Path) -> Result<()> {
    let source = source
        .canonicalize()
        .with_context(|| format!("Failed to resolve {}", source.display()))?;
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("Skill destination has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("Failed to create {}", parent.display()))?;
    fs::create_dir(destination)
        .with_context(|| format!("Failed to create {}", destination.display()))?;

    let result = (|| {
        for entry in WalkDir::new(&source).min_depth(1).follow_links(false) {
            let entry = entry?;
            if entry.path().is_symlink() {
                return Err(anyhow!(
                    "Skill contains an unsupported symbolic link: {}",
                    entry.path().display()
                ));
            }
            let relative = entry.path().strip_prefix(&source)?;
            let target = destination.join(relative);
            if entry.file_type().is_dir() {
                fs::create_dir(&target)?;
            } else if entry.file_type().is_file() {
                fs::copy(entry.path(), &target)?;
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn uninstall(agents: &[SkillAgent], request: &SkillMutation) -> Result<SkillsOverview> {
    let overview = discover(agents);
    let skill = overview
        .skills
        .iter()
        .find(|skill| skill.id == request.skill_id)
        .ok_or_else(|| anyhow!("Unknown skill: {}", request.skill_id))?;
    let agent = agents
        .iter()
        .find(|agent| agent.provider_id == request.provider)
        .ok_or_else(|| anyhow!("Unknown skill provider: {}", request.provider))?;
    let installation = skill
        .installations
        .iter()
        .find(|installation| installation.provider_id == request.provider)
        .ok_or_else(|| anyhow!("Skill is not installed for {}", agent.name))?;
    let directory = installation
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("Skill installation has no directory name"))?;
    validate_directory(directory)?;
    let expected = agent.skills_dir.join(directory);
    if installation.path != expected || !expected.join(MANAGED_MARKER).is_file() {
        return Err(anyhow!("Refusing to remove a user-owned skill"));
    }
    fs::remove_dir_all(&expected)
        .with_context(|| format!("Failed to remove {}", expected.display()))?;
    Ok(discover(agents))
}

async fn list_skills(State(state): State<SkillsState>) -> impl IntoResponse {
    ApiResponse::success(discover(&state.agents)).into_response()
}

async fn install_skill(
    State(state): State<SkillsState>,
    Json(request): Json<SkillMutation>,
) -> impl IntoResponse {
    match install(&state.agents, &request) {
        Ok(overview) => ApiResponse::success(overview).into_response(),
        Err(error) => error_response(error),
    }
}

async fn uninstall_skill(
    State(state): State<SkillsState>,
    Json(request): Json<SkillMutation>,
) -> impl IntoResponse {
    match uninstall(&state.agents, &request) {
        Ok(overview) => ApiResponse::success(overview).into_response(),
        Err(error) => error_response(error),
    }
}

fn error_response(error: anyhow::Error) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiResponse::<()> {
            ok: false,
            data: None,
            error: Some(error.to_string()),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use serde_json::Value;
    use tower::ServiceExt;

    fn agents(root: &Path) -> Vec<SkillAgent> {
        [
            ("claude", "Claude Code"),
            ("codex", "Codex"),
            ("gemini", "Gemini CLI"),
        ]
        .into_iter()
        .map(|(provider_id, name)| SkillAgent {
            provider_id: provider_id.into(),
            name: name.into(),
            skills_dir: root.join(provider_id).join("skills"),
        })
        .collect()
    }

    fn create_skill(root: &Path, provider: &str, directory: &str, contents: &str) -> PathBuf {
        let path = root.join(provider).join("skills").join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), contents).unwrap();
        path
    }

    async fn json(app: Router, request: Request<Body>) -> (StatusCode, Value) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[test]
    fn discovery_parses_frontmatter_and_merges_installations() {
        let root = tempfile::tempdir().unwrap();
        create_skill(
            root.path(),
            "claude",
            "writer",
            "---\nname: Document Writer\ndescription: Writes concise docs\n---\n# Skill",
        );
        create_skill(
            root.path(),
            "codex",
            "document-writer",
            "---\nname: Document Writer\n---\n",
        );
        create_skill(root.path(), "gemini", "fallback", "# No frontmatter");

        let overview = discover(&agents(root.path()));
        assert_eq!(overview.skills.len(), 2);
        let writer = overview
            .skills
            .iter()
            .find(|skill| skill.id == "document-writer")
            .unwrap();
        assert_eq!(writer.description.as_deref(), Some("Writes concise docs"));
        assert_eq!(writer.installations.len(), 2);
        assert!(overview.skills.iter().any(|skill| skill.name == "fallback"));
    }

    #[test]
    fn install_copies_skill_marks_it_and_rejects_duplicates() {
        let root = tempfile::tempdir().unwrap();
        let source = create_skill(root.path(), "claude", "writer", "---\nname: Writer\n---\n");
        fs::write(source.join("example.txt"), "example").unwrap();
        let agents = agents(root.path());
        let request = SkillMutation {
            skill_id: "writer".into(),
            provider: "codex".into(),
        };

        let overview = install(&agents, &request).unwrap();
        let installed = root.path().join("codex/skills/writer");
        assert_eq!(
            fs::read_to_string(installed.join("example.txt")).unwrap(),
            "example"
        );
        assert!(installed.join(MANAGED_MARKER).is_file());
        assert_eq!(overview.skills[0].installations.len(), 2);
        assert!(install(&agents, &request).is_err());
    }

    #[test]
    fn uninstall_removes_managed_copy_but_refuses_user_owned_skill() {
        let root = tempfile::tempdir().unwrap();
        create_skill(root.path(), "claude", "writer", "# Writer");
        let agents = agents(root.path());
        let managed = SkillMutation {
            skill_id: "writer".into(),
            provider: "codex".into(),
        };
        install(&agents, &managed).unwrap();
        uninstall(&agents, &managed).unwrap();
        assert!(!root.path().join("codex/skills/writer").exists());

        let user_owned = SkillMutation {
            skill_id: "writer".into(),
            provider: "claude".into(),
        };
        assert!(uninstall(&agents, &user_owned).is_err());
        assert!(root.path().join("claude/skills/writer").exists());
    }

    #[tokio::test]
    async fn api_lists_installs_and_removes_skills() {
        let root = tempfile::tempdir().unwrap();
        create_skill(root.path(), "claude", "writer", "# Writer");
        let app = router_for(agents(root.path()));

        let (status, listed) = json(
            app.clone(),
            Request::builder()
                .uri("/api/v1/skills")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["data"]["skills"].as_array().unwrap().len(), 1);

        let request = serde_json::to_vec(&SkillMutation {
            skill_id: "writer".into(),
            provider: "codex".into(),
        })
        .unwrap();
        let (status, installed) = json(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/skills/install")
                .header("content-type", "application/json")
                .body(Body::from(request.clone()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            installed["data"]["skills"][0]["installations"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        let (status, removed) = json(
            app,
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/skills/install")
                .header("content-type", "application/json")
                .body(Body::from(request))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            removed["data"]["skills"][0]["installations"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
