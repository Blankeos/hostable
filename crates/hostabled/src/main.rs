mod backup;
mod executor;

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use hostable_core::{
    ActionPlan, ArkSurvivalEvolvedAdapter, CreateServerInput, GameAdapter, GameKind,
    MinecraftAdapter, ModrinthProjectType, ModrinthSearchResponse, ModrinthVersion, RegistryStore,
    ServerId, ServerRecord,
};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Debug, Parser)]
#[command(name = "hostabled")]
#[command(about = "Hostable local daemon")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: SocketAddr,

    #[arg(long, default_value = "/var/lib/hostable/registry.sqlite")]
    registry: PathBuf,

    #[arg(long, default_value = "/srv/hostable/backups")]
    backup_root: PathBuf,

    #[arg(long, default_value = "/var/lib/hostable/web-admin")]
    admin_dist: PathBuf,
}

#[derive(Clone)]
struct AppState {
    registry: RegistryStore,
    backup_root: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let registry = RegistryStore::open(&args.registry)
        .with_context(|| format!("failed to open registry at {}", args.registry.display()))?;
    let state = AppState {
        registry,
        backup_root: args.backup_root,
    };

    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/adapters", get(adapters))
        .route("/api/modrinth/search", get(modrinth_search))
        .route(
            "/api/modrinth/projects/{id}/versions",
            get(modrinth_versions),
        )
        .route("/api/servers/plan", post(plan_server))
        .route("/api/servers/create", post(create_server))
        .route("/api/servers", get(list_servers).post(register_server))
        .route("/api/servers/{id}", get(get_server))
        .route("/api/servers/{id}/status", get(server_status))
        .route("/api/servers/{id}/start", post(start_server))
        .route("/api/servers/{id}/stop", post(stop_server))
        .route("/api/servers/{id}/backups/run", post(run_backup))
        .with_state(state);
    let app = if args.admin_dist.join("index.html").exists() {
        api.fallback_service(
            ServeDir::new(&args.admin_dist)
                .not_found_service(ServeFile::new(args.admin_dist.join("index.html"))),
        )
    } else {
        api.route("/", get(root))
    };

    let listener = TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("failed to bind daemon HTTP listener on {}", args.bind))?;

    println!("hostabled listening on http://{}", args.bind);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn root() -> &'static str {
    "hostabled is running\n"
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "hostabled",
    })
}

async fn adapters() -> Json<Vec<AdapterSummary>> {
    let minecraft = MinecraftAdapter;
    let ark = ArkSurvivalEvolvedAdapter;

    Json(vec![adapter_summary(&minecraft), adapter_summary(&ark)])
}

async fn modrinth_search(
    Query(query): Query<ModrinthSearchQuery>,
) -> Result<Json<ModrinthSearchResponse>, ApiError> {
    let facets = modrinth_facets(&query);
    let mut response = reqwest::Client::new()
        .get("https://api.modrinth.com/v2/search")
        .header("User-Agent", "hostable/0.1.0")
        .query(&[
            ("query", query.query.as_deref().unwrap_or_default()),
            ("facets", facets.as_str()),
            ("limit", "20"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json::<ModrinthSearchResponse>()
        .await?;
    response
        .hits
        .retain(|hit| hit.server_side.as_deref() != Some("unsupported"));

    Ok(Json(response))
}

async fn modrinth_versions(
    Path(id): Path<String>,
    Query(query): Query<ModrinthVersionsQuery>,
) -> Result<Json<Vec<ModrinthVersion>>, ApiError> {
    let loaders = query.loader.map(|loader| format!(r#"["{loader}"]"#));
    let game_versions = query
        .game_version
        .map(|version| format!(r#"["{version}"]"#));
    let mut request = reqwest::Client::new()
        .get(format!("https://api.modrinth.com/v2/project/{id}/version"))
        .header("User-Agent", "hostable/0.1.0");

    if let Some(loaders) = &loaders {
        request = request.query(&[("loaders", loaders)]);
    }
    if let Some(game_versions) = &game_versions {
        request = request.query(&[("game_versions", game_versions)]);
    }

    let versions = request
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<ModrinthVersion>>()
        .await?;

    Ok(Json(versions))
}

async fn list_servers(State(state): State<AppState>) -> Result<Json<Vec<ServerRecord>>, ApiError> {
    Ok(Json(state.registry.list_servers()?))
}

async fn get_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ServerRecord>, ApiError> {
    let id = ServerId(id);
    state
        .registry
        .get_server(&id)?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

async fn register_server(
    State(state): State<AppState>,
    Json(server): Json<ServerRecord>,
) -> Result<Json<ServerRecord>, ApiError> {
    state.registry.upsert_server(&server)?;
    Ok(Json(server))
}

async fn plan_server(
    Json(request): Json<CreateServerRequest>,
) -> Result<Json<ActionPlan>, ApiError> {
    let plan = match request.game {
        GameKind::Minecraft => MinecraftAdapter.plan_install(request.input)?,
        GameKind::ArkSurvivalEvolved => ArkSurvivalEvolvedAdapter.plan_install(request.input)?,
    };
    Ok(Json(plan))
}

async fn create_server(
    State(state): State<AppState>,
    Json(request): Json<CreateServerRequest>,
) -> Result<Json<CreateServerResponse>, ApiError> {
    let game = request.game;
    let input = request.input;
    let dry_run = request.dry_run;
    let plan = match game {
        GameKind::Minecraft => MinecraftAdapter.plan_install(input.clone())?,
        GameKind::ArkSurvivalEvolved => ArkSurvivalEvolvedAdapter.plan_install(input.clone())?,
    };
    let executor = executor::ActionExecutor::new(dry_run);
    let log = executor.execute(&plan.actions)?;
    let server = server_record_from_input(game, &input);
    if !dry_run {
        state.registry.upsert_server(&server)?;
    }
    Ok(Json(CreateServerResponse { server, plan, log }))
}

async fn server_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ServiceStatusResponse>, ApiError> {
    let server = state
        .registry
        .get_server(&ServerId(id))?
        .ok_or(ApiError::NotFound)?;
    let active = executor::systemctl_is_active(&server.service_name)?;
    Ok(Json(ServiceStatusResponse { active }))
}

async fn start_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ServiceActionResponse>, ApiError> {
    let server = state
        .registry
        .get_server(&ServerId(id))?
        .ok_or(ApiError::NotFound)?;
    executor::systemctl(&server.service_name, "start")?;
    Ok(Json(ServiceActionResponse {
        ok: true,
        action: "start",
    }))
}

async fn stop_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ServiceActionResponse>, ApiError> {
    let server = state
        .registry
        .get_server(&ServerId(id))?
        .ok_or(ApiError::NotFound)?;
    executor::systemctl(&server.service_name, "stop")?;
    Ok(Json(ServiceActionResponse {
        ok: true,
        action: "stop",
    }))
}

async fn run_backup(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<backup::BackupRunResult>, ApiError> {
    let server = state
        .registry
        .get_server(&ServerId(id))?
        .ok_or(ApiError::NotFound)?;
    let runner = backup::BackupRunner::new(state.backup_root);
    Ok(Json(runner.run(&server)?))
}

fn adapter_summary(adapter: &dyn GameAdapter) -> AdapterSummary {
    AdapterSummary {
        kind: adapter.kind(),
        display_name: adapter.display_name(),
        default_ports: adapter.default_ports(),
        has_console: adapter.console().is_some(),
        has_modrinth: adapter.modrinth().is_some(),
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    service: &'static str,
}

#[derive(Debug, Serialize)]
struct AdapterSummary {
    kind: GameKind,
    display_name: &'static str,
    default_ports: Vec<hostable_core::PortSpec>,
    has_console: bool,
    has_modrinth: bool,
}

#[derive(Debug, Deserialize)]
struct CreateServerRequest {
    game: GameKind,
    input: CreateServerInput,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
struct ModrinthSearchQuery {
    query: Option<String>,
    project_type: ModrinthProjectType,
    game_version: Option<String>,
    loader: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModrinthVersionsQuery {
    game_version: Option<String>,
    loader: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateServerResponse {
    server: ServerRecord,
    plan: ActionPlan,
    log: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ServiceStatusResponse {
    active: String,
}

#[derive(Debug, Serialize)]
struct ServiceActionResponse {
    ok: bool,
    action: &'static str,
}

fn server_record_from_input(game: GameKind, input: &CreateServerInput) -> ServerRecord {
    let id = slugify(&input.name);
    let ports = match game {
        GameKind::Minecraft => MinecraftAdapter.default_ports(),
        GameKind::ArkSurvivalEvolved => ArkSurvivalEvolvedAdapter.default_ports(),
    };
    ServerRecord {
        id: ServerId(id.clone()),
        name: input.name.clone(),
        game,
        install_path: input.install_path.clone(),
        service_name: format!("hostable-{id}.service"),
        ports,
        adapter_config: serde_json::json!({
            "server_type": input.server_type,
            "game_version": input.game_version,
            "loader_version": input.loader_version,
            "build_version": input.build_version,
            "memory_mb": input.memory_mb,
            "extra": input.extra,
        }),
    }
}

fn slugify(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn modrinth_facets(query: &ModrinthSearchQuery) -> String {
    let mut facets = vec![vec![query.project_type.as_facet().to_string()]];
    if let Some(game_version) = &query.game_version {
        facets.push(vec![format!("versions:{game_version}")]);
    }
    if let Some(loader) = &query.loader {
        facets.push(vec![format!("categories:{loader}")]);
    }
    serde_json::to_string(&facets).expect("facets serialize")
}

#[derive(Debug)]
enum ApiError {
    NotFound,
    Internal(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "not_found" })),
            )
                .into_response(),
            ApiError::Internal(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response(),
        }
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        ApiError::Internal(error.into())
    }
}
