mod auth;
mod backup;
mod executor;
mod s3;
mod settings;

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use clap::Parser;
use hostable_core::{
    ActionPlan, ArkSurvivalEvolvedAdapter, ConfigDocument, ConfigPatch, CreateServerInput,
    GameAdapter, GameKind, MinecraftAdapter, ModrinthInstallFile, ModrinthInstallSelection,
    ModrinthProjectType, ModrinthSearchResponse, ModrinthVersion, RegistryStore, S3Config,
    ServerId, ServerRecord,
};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Component, PathBuf},
};
use tokio::net::TcpListener;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "hostabled")]
#[command(about = "Hostable local daemon")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8080")]
    bind: SocketAddr,

    #[arg(long, default_value = "/var/lib/hostable/registry.sqlite")]
    registry: PathBuf,

    #[arg(long, default_value = "/srv/hostable/backups")]
    backup_root: PathBuf,

    #[arg(long, default_value = "/etc/hostable/settings.json")]
    settings: PathBuf,

    #[arg(long, default_value = "/var/lib/hostable/web-admin")]
    admin_dist: PathBuf,

    #[arg(long, default_value = "/etc/hostable/admin-password.hash")]
    admin_password_hash: PathBuf,

    #[arg(long, default_value = "/etc/hostable/operator-token")]
    operator_token: PathBuf,
}

#[derive(Clone)]
struct AppState {
    registry: RegistryStore,
    backup_root: PathBuf,
    settings: settings::SettingsStore,
    auth: auth::AuthStore,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let registry = RegistryStore::open(&args.registry)
        .with_context(|| format!("failed to open registry at {}", args.registry.display()))?;
    let state = AppState {
        registry,
        backup_root: args.backup_root,
        settings: settings::SettingsStore::new(args.settings),
        auth: auth::AuthStore::new(args.admin_password_hash, args.operator_token),
    };

    let public_api = Router::new()
        .route("/api/health", get(health))
        .route("/api/adapters", get(adapters))
        .route("/api/admin/me", get(admin_me))
        .route("/api/admin/setup", post(admin_setup))
        .route("/api/admin/login", post(admin_login))
        .route("/api/admin/logout", post(admin_logout));

    let protected_api = Router::new()
        .route("/api/modrinth/search", get(modrinth_search))
        .route(
            "/api/modrinth/projects/{id}/versions",
            get(modrinth_versions),
        )
        .route(
            "/api/modrinth/projects/{id}/install",
            get(modrinth_install_selection),
        )
        .route("/api/servers/plan", post(plan_server))
        .route("/api/servers/create", post(create_server))
        .route("/api/servers/import", post(import_server))
        .route("/api/servers", get(list_servers).post(register_server))
        .route("/api/servers/{id}", get(get_server))
        .route("/api/servers/{id}/status", get(server_status))
        .route("/api/servers/{id}/logs", get(server_logs))
        .route(
            "/api/servers/{id}/config",
            get(read_server_config).put(write_server_config),
        )
        .route("/api/servers/{id}/start", post(start_server))
        .route("/api/servers/{id}/stop", post(stop_server))
        .route("/api/servers/{id}/restart", post(restart_server))
        .route("/api/servers/{id}/console", post(send_console_command))
        .route("/api/servers/{id}/backups/run", post(run_backup))
        .route("/api/settings", get(get_settings))
        .route("/api/settings/backups", put(update_backup_settings))
        .route("/api/settings/s3", put(update_s3_settings))
        .route("/api/settings/s3/test", post(test_s3_settings))
        .route("/api/settings/admin-web", put(update_admin_web_settings))
        .route("/api/admin/password", put(admin_change_password))
        .route("/api/network/public-ip", get(public_ip))
        .route_layer(middleware::from_fn_with_state(
            state.auth.clone(),
            auth::require_auth,
        ));
    let api = public_api.merge(protected_api);
    let admin_dist = args.admin_dist.clone();
    let app = if args.admin_dist.join("index.html").exists() {
        api.fallback(move |State(state): State<AppState>, uri: Uri| {
            serve_admin_spa(state, uri, admin_dist.clone())
        })
    } else {
        api.route("/", get(root))
    }
    .with_state(state.clone());

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

async fn serve_admin_spa(state: AppState, uri: Uri, admin_dist: PathBuf) -> Response {
    let Ok(settings) = state.settings.load() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to load hostable settings\n",
        )
            .into_response();
    };
    if !settings.admin_web.enabled {
        return (
            StatusCode::NOT_FOUND,
            "hostable admin web is disabled; enable it in the TUI settings\n",
        )
            .into_response();
    }

    let requested = uri.path().trim_start_matches('/');
    let relative = if requested.is_empty() {
        PathBuf::from("index.html")
    } else {
        PathBuf::from(requested)
    };
    if relative
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return StatusCode::NOT_FOUND.into_response();
    }

    let requested_path = admin_dist.join(&relative);
    let path = if requested_path.is_file() {
        requested_path
    } else {
        admin_dist.join("index.html")
    };
    let content_type = content_type_for_path(&path);
    match std::fs::read(&path) {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
            bytes,
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read admin asset: {error}\n"),
        )
            .into_response(),
    }
}

fn content_type_for_path(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
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

async fn admin_me(State(state): State<AppState>, headers: HeaderMap) -> Json<AdminMeResponse> {
    Json(AdminMeResponse {
        setup_required: state.auth.setup_required(),
        authenticated: state.auth.authenticated_headers(&headers),
        admin_web_enabled: state
            .settings
            .load()
            .map(|settings| settings.admin_web.enabled)
            .unwrap_or(false),
    })
}

async fn admin_setup(
    State(state): State<AppState>,
    Json(request): Json<PasswordRequest>,
) -> Result<Response, ApiError> {
    require_admin_web_enabled(&state)?;
    if !state.auth.setup_required() {
        return Err(ApiError::Conflict("admin password is already configured"));
    }

    state.auth.set_password(&request.password)?;
    Ok(authenticated_response(&state.auth))
}

async fn admin_login(
    State(state): State<AppState>,
    Json(request): Json<PasswordRequest>,
) -> Result<Response, ApiError> {
    require_admin_web_enabled(&state)?;
    if state.auth.setup_required() {
        return Err(ApiError::BadRequest("admin password is not configured"));
    }

    if !state.auth.verify_password(&request.password)? {
        return Err(ApiError::Unauthorized);
    }

    Ok(authenticated_response(&state.auth))
}

async fn admin_logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = auth::session_from_headers(&headers) {
        state.auth.remove_session(&token);
    }

    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth::clear_session_cookie()).expect("valid cookie header"),
    );
    response
}

async fn admin_change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<Response, ApiError> {
    if request.new_password.len() < 10 {
        return Err(ApiError::BadRequest(
            "admin password must be at least 10 characters",
        ));
    }
    let operator_authenticated = state.auth.operator_authenticated_headers(&headers);
    if !operator_authenticated {
        require_admin_web_enabled(&state)?;
        if state.auth.setup_required() {
            return Err(ApiError::BadRequest("admin password is not configured"));
        }
        let Some(current_password) = request.current_password.as_deref() else {
            return Err(ApiError::Unauthorized);
        };
        if !state.auth.verify_password(current_password)? {
            return Err(ApiError::Unauthorized);
        }
    }
    state.auth.set_password(&request.new_password)?;
    state.auth.clear_sessions();
    Ok(authenticated_response(&state.auth))
}

async fn modrinth_search(
    Query(query): Query<ModrinthSearchQuery>,
) -> Result<Json<ModrinthSearchResponse>, ApiError> {
    let facets = modrinth_facets(&query);
    let search = query
        .query
        .as_deref()
        .or(query.q.as_deref())
        .unwrap_or_default();
    let mut response = reqwest::Client::new()
        .get("https://api.modrinth.com/v2/search")
        .header("User-Agent", "hostable/0.1.0")
        .query(&[
            ("query", search),
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

async fn modrinth_install_selection(
    Path(id): Path<String>,
    Query(query): Query<ModrinthInstallQuery>,
) -> Result<Json<ModrinthInstallSelection>, ApiError> {
    let client = reqwest::Client::new();
    let version =
        select_modrinth_version(&client, &id, &query.game_version, query.loader.as_deref()).await?;
    let file = install_file_from_version(&version).ok_or(ApiError::NotFound)?;
    let mut dependencies = Vec::new();

    if query.project_type == ModrinthProjectType::Mod {
        for dependency in version
            .dependencies
            .iter()
            .filter(|dependency| dependency.dependency_type == "required")
        {
            if dependencies.len() >= 16 {
                break;
            }
            let dependency_version = if let Some(version_id) = &dependency.version_id {
                fetch_modrinth_version(&client, version_id).await?
            } else if let Some(project_id) = &dependency.project_id {
                select_modrinth_version(
                    &client,
                    project_id,
                    &query.game_version,
                    query.loader.as_deref(),
                )
                .await?
            } else {
                continue;
            };
            if let Some(file) = install_file_from_version(&dependency_version) {
                dependencies.push(file);
            }
        }
    }

    Ok(Json(ModrinthInstallSelection {
        project_id: id,
        project_type: query.project_type,
        version_id: version.id,
        version_number: version.version_number,
        file,
        dependencies,
    }))
}

async fn select_modrinth_version(
    client: &reqwest::Client,
    project_id: &str,
    game_version: &str,
    loader: Option<&str>,
) -> Result<ModrinthVersion, ApiError> {
    let loaders = loader.map(|loader| format!(r#"["{loader}"]"#));
    let game_versions = format!(r#"["{game_version}"]"#);
    let mut request = client
        .get(format!(
            "https://api.modrinth.com/v2/project/{project_id}/version"
        ))
        .header("User-Agent", "hostable/0.1.0")
        .query(&[("game_versions", game_versions.as_str())]);
    if let Some(loaders) = &loaders {
        request = request.query(&[("loaders", loaders)]);
    }

    let versions = request
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<ModrinthVersion>>()
        .await?;
    versions
        .into_iter()
        .find(|version| install_file_from_version(version).is_some())
        .ok_or(ApiError::NotFound)
}

async fn fetch_modrinth_version(
    client: &reqwest::Client,
    version_id: &str,
) -> Result<ModrinthVersion, ApiError> {
    Ok(client
        .get(format!("https://api.modrinth.com/v2/version/{version_id}"))
        .header("User-Agent", "hostable/0.1.0")
        .send()
        .await?
        .error_for_status()?
        .json::<ModrinthVersion>()
        .await?)
}

fn install_file_from_version(version: &ModrinthVersion) -> Option<ModrinthInstallFile> {
    let file = version
        .files
        .iter()
        .find(|file| file.primary)
        .or_else(|| version.files.first())?;
    Some(ModrinthInstallFile {
        project_id: version.project_id.clone(),
        version_id: version.id.clone(),
        filename: file.filename.clone(),
        url: file.url.clone(),
        sha512: file.hashes.sha512.clone(),
        size: file.size,
    })
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
    let input = prepare_create_input(request.game, request.input);
    let plan = match request.game {
        GameKind::Minecraft => MinecraftAdapter.plan_install(input)?,
        GameKind::ArkSurvivalEvolved => ArkSurvivalEvolvedAdapter.plan_install(input)?,
    };
    Ok(Json(plan))
}

async fn create_server(
    State(state): State<AppState>,
    Json(request): Json<CreateServerRequest>,
) -> Result<Json<CreateServerResponse>, ApiError> {
    let game = request.game;
    let input = prepare_create_input(game, request.input);
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

async fn import_server(
    State(state): State<AppState>,
    Json(request): Json<ImportServerRequest>,
) -> Result<Json<CreateServerResponse>, ApiError> {
    let detected = match request.game {
        GameKind::Minecraft => MinecraftAdapter.detect_existing(&request.install_path)?,
        GameKind::ArkSurvivalEvolved => {
            ArkSurvivalEvolvedAdapter.detect_existing(&request.install_path)?
        }
    };
    let plan = match request.game {
        GameKind::Minecraft => MinecraftAdapter.plan_import(detected.clone())?,
        GameKind::ArkSurvivalEvolved => ArkSurvivalEvolvedAdapter.plan_import(detected.clone())?,
    };
    let executor = executor::ActionExecutor::new(request.dry_run);
    let log = executor.execute(&plan.actions)?;
    let server = server_record_from_import(request.game, &request.name, detected);
    if !request.dry_run {
        state.registry.upsert_server(&server)?;
    }
    Ok(Json(CreateServerResponse { server, plan, log }))
}

fn prepare_create_input(game: GameKind, mut input: CreateServerInput) -> CreateServerInput {
    let mut extra = input
        .extra
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);
    match game {
        GameKind::Minecraft => {
            extra
                .entry("rcon_password".to_string())
                .or_insert_with(|| serde_json::Value::String(format!("rcon-{}", Uuid::new_v4())));
        }
        GameKind::ArkSurvivalEvolved => {
            extra
                .entry("admin_password".to_string())
                .or_insert_with(|| serde_json::Value::String(format!("admin-{}", Uuid::new_v4())));
        }
    }
    input.extra = serde_json::Value::Object(extra);
    input
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

async fn server_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<LogsResponse>, ApiError> {
    let server = state
        .registry
        .get_server(&ServerId(id))?
        .ok_or(ApiError::NotFound)?;
    let lines = query.lines.unwrap_or(120).clamp(1, 500);
    let text = executor::journalctl_logs(&server.service_name, lines)?;
    Ok(Json(LogsResponse {
        service: server.service_name,
        lines,
        text,
    }))
}

async fn read_server_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ConfigDocument>, ApiError> {
    let server = state
        .registry
        .get_server(&ServerId(id))?
        .ok_or(ApiError::NotFound)?;
    let config = match server.game {
        GameKind::Minecraft => MinecraftAdapter.read_config(&server)?,
        GameKind::ArkSurvivalEvolved => ArkSurvivalEvolvedAdapter.read_config(&server)?,
    };
    Ok(Json(config))
}

async fn write_server_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(patch): Json<ConfigPatch>,
) -> Result<Json<ConfigDocument>, ApiError> {
    let server = state
        .registry
        .get_server(&ServerId(id))?
        .ok_or(ApiError::NotFound)?;
    match server.game {
        GameKind::Minecraft => MinecraftAdapter.write_config(&server, patch)?,
        GameKind::ArkSurvivalEvolved => ArkSurvivalEvolvedAdapter.write_config(&server, patch)?,
    }
    read_server_config(State(state), Path(server.id.0)).await
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

async fn restart_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ServiceActionResponse>, ApiError> {
    let server = state
        .registry
        .get_server(&ServerId(id))?
        .ok_or(ApiError::NotFound)?;
    executor::systemctl(&server.service_name, "restart")?;
    Ok(Json(ServiceActionResponse {
        ok: true,
        action: "restart",
    }))
}

async fn send_console_command(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ConsoleCommandRequest>,
) -> Result<Json<hostable_core::adapter::ConsoleCommandResult>, ApiError> {
    let server = state
        .registry
        .get_server(&ServerId(id))?
        .ok_or(ApiError::NotFound)?;
    let result = match server.game {
        GameKind::Minecraft => MinecraftAdapter
            .console()
            .ok_or(ApiError::BadRequest("minecraft console is not available"))?
            .send_command(&server, &request.command)?,
        GameKind::ArkSurvivalEvolved => ArkSurvivalEvolvedAdapter
            .console()
            .ok_or(ApiError::BadRequest("ark console is not available"))?
            .send_command(&server, &request.command)?,
    };
    Ok(Json(result))
}

async fn run_backup(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<backup::BackupRunResult>, ApiError> {
    let server = state
        .registry
        .get_server(&ServerId(id))?
        .ok_or(ApiError::NotFound)?;
    let settings = state.settings.load()?;
    let runner = backup::BackupRunner::new(state.backup_root);
    let mut result = runner.run_with_retention(&server, settings.backup.local_keep_last)?;

    if let Some(config) = settings.s3 {
        let client = s3::S3Client::from_env(config)?;
        let archive_name = result
            .archive_path
            .file_name()
            .and_then(|name| name.to_str())
            .context("backup archive path is missing a file name")?;
        let manifest_name = result
            .manifest_path
            .file_name()
            .and_then(|name| name.to_str())
            .context("backup manifest path is missing a file name")?;
        let archive_upload = client
            .put_file(
                &format!("{}/{}", server.id.0, archive_name),
                &result.archive_path,
                "application/zstd",
            )
            .await?;
        let manifest_upload = client
            .put_file(
                &format!("{}/{}", server.id.0, manifest_name),
                &result.manifest_path,
                "application/json",
            )
            .await?;
        result.remote_artifacts.push(backup::RemoteBackupArtifact {
            provider: "s3".to_string(),
            key: archive_upload.key,
            bytes: archive_upload.bytes,
        });
        result.remote_artifacts.push(backup::RemoteBackupArtifact {
            provider: "s3".to_string(),
            key: manifest_upload.key,
            bytes: manifest_upload.bytes,
        });
        for deleted in client
            .prune_backup_sets(&server.id.0, settings.backup.s3_keep_last)
            .await?
        {
            result
                .pruned_remote_artifacts
                .push(backup::RemoteBackupArtifact {
                    provider: "s3".to_string(),
                    key: deleted.key,
                    bytes: 0,
                });
        }
    }

    Ok(Json(result))
}

async fn get_settings(
    State(state): State<AppState>,
) -> Result<Json<settings::HostableSettings>, ApiError> {
    Ok(Json(state.settings.load()?))
}

async fn update_s3_settings(
    State(state): State<AppState>,
    Json(config): Json<S3Config>,
) -> Result<Json<settings::HostableSettings>, ApiError> {
    validate_s3_config(&config)?;
    let mut settings = state.settings.load()?;
    settings.s3 = Some(config);
    state.settings.save(&settings)?;
    Ok(Json(settings))
}

async fn update_backup_settings(
    State(state): State<AppState>,
    Json(config): Json<settings::BackupSettings>,
) -> Result<Json<settings::HostableSettings>, ApiError> {
    validate_backup_settings(&config)?;
    let mut settings = state.settings.load()?;
    settings.backup = config;
    state.settings.save(&settings)?;
    Ok(Json(settings))
}

async fn update_admin_web_settings(
    State(state): State<AppState>,
    Json(config): Json<settings::AdminWebSettings>,
) -> Result<Json<settings::HostableSettings>, ApiError> {
    if let Some(domain) = &config.domain {
        validate_domain(domain)?;
    }
    let mut settings = state.settings.load()?;
    settings.admin_web = config;
    state.settings.save(&settings)?;
    Ok(Json(settings))
}

async fn test_s3_settings(
    State(state): State<AppState>,
) -> Result<Json<s3::S3PutResult>, ApiError> {
    let config = state
        .settings
        .load()?
        .s3
        .ok_or(ApiError::BadRequest("s3 is not configured"))?;
    let client = s3::S3Client::from_env(config)?;
    Ok(Json(client.test_connection().await?))
}

async fn public_ip() -> Result<Json<PublicIpResponse>, ApiError> {
    let ip = reqwest::Client::new()
        .get("https://api.ipify.org")
        .header("User-Agent", "hostable/0.1.0")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(Json(PublicIpResponse {
        ip: ip.trim().to_string(),
    }))
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

#[derive(Debug, Serialize)]
struct AdminMeResponse {
    setup_required: bool,
    authenticated: bool,
    admin_web_enabled: bool,
}

#[derive(Debug, Deserialize)]
struct PasswordRequest {
    password: String,
}

#[derive(Debug, Deserialize)]
struct ChangePasswordRequest {
    current_password: Option<String>,
    new_password: String,
}

#[derive(Debug, Deserialize)]
struct CreateServerRequest {
    game: GameKind,
    input: CreateServerInput,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
struct ImportServerRequest {
    game: GameKind,
    name: String,
    install_path: PathBuf,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
struct ModrinthSearchQuery {
    query: Option<String>,
    q: Option<String>,
    project_type: ModrinthProjectType,
    game_version: Option<String>,
    loader: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModrinthVersionsQuery {
    game_version: Option<String>,
    loader: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModrinthInstallQuery {
    project_type: ModrinthProjectType,
    game_version: String,
    loader: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    lines: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct ConsoleCommandRequest {
    command: String,
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

#[derive(Debug, Serialize, Deserialize)]
struct LogsResponse {
    service: String,
    lines: u16,
    text: String,
}

#[derive(Debug, Serialize)]
struct PublicIpResponse {
    ip: String,
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

fn server_record_from_import(
    game: GameKind,
    name: &str,
    detected: hostable_core::DetectedServer,
) -> ServerRecord {
    let id = slugify(name);
    let ports = match game {
        GameKind::Minecraft => MinecraftAdapter.default_ports(),
        GameKind::ArkSurvivalEvolved => ArkSurvivalEvolvedAdapter.default_ports(),
    };
    ServerRecord {
        id: ServerId(id.clone()),
        name: name.to_string(),
        game,
        install_path: detected.install_path,
        service_name: format!("hostable-{id}.service"),
        ports,
        adapter_config: serde_json::json!({
            "server_type": detected.server_type,
            "imported": true,
        }),
    }
}

fn validate_s3_config(config: &S3Config) -> Result<(), ApiError> {
    if config.bucket.trim().is_empty() {
        return Err(ApiError::BadRequest("bucket is required"));
    }
    if config.prefix.contains("..") {
        return Err(ApiError::BadRequest("prefix cannot contain .."));
    }
    if config.access_key_env.trim().is_empty() || config.secret_key_env.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "access_key_env and secret_key_env are required",
        ));
    }
    Ok(())
}

fn validate_backup_settings(config: &settings::BackupSettings) -> Result<(), ApiError> {
    if config.local_keep_last == 0 || config.local_keep_last > 100 {
        return Err(ApiError::BadRequest(
            "local_keep_last must be between 1 and 100",
        ));
    }
    if config.s3_keep_last == 0 || config.s3_keep_last > 100 {
        return Err(ApiError::BadRequest(
            "s3_keep_last must be between 1 and 100",
        ));
    }
    Ok(())
}

fn validate_domain(domain: &str) -> Result<(), ApiError> {
    let domain = domain.trim();
    if domain.is_empty() {
        return Ok(());
    }
    if domain.len() > 253 {
        return Err(ApiError::BadRequest("domain contains invalid characters"));
    }
    for label in domain.split('.') {
        let valid_label = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-');
        if !valid_label {
            return Err(ApiError::BadRequest("domain contains invalid characters"));
        }
    }
    Ok(())
}

fn require_admin_web_enabled(state: &AppState) -> Result<(), ApiError> {
    if state.settings.load()?.admin_web.enabled {
        Ok(())
    } else {
        Err(ApiError::Forbidden("admin web is disabled"))
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
    BadRequest(&'static str),
    Conflict(&'static str),
    Forbidden(&'static str),
    NotFound,
    Unauthorized,
    Internal(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": message })),
            )
                .into_response(),
            ApiError::Conflict(message) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": message })),
            )
                .into_response(),
            ApiError::Forbidden(message) => (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": message })),
            )
                .into_response(),
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "not_found" })),
            )
                .into_response(),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "unauthorized" })),
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

fn authenticated_response(auth: &auth::AuthStore) -> Response {
    let token = auth.create_session();
    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&auth::session_cookie(&token)).expect("valid cookie header"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_domain_labels() {
        assert!(validate_domain("").is_ok());
        assert!(validate_domain("play.example.com").is_ok());
        assert!(validate_domain("play-server.example.co").is_ok());

        assert!(validate_domain("-play.example.com").is_err());
        assert!(validate_domain("play-.example.com").is_err());
        assert!(validate_domain("play..example.com").is_err());
        assert!(validate_domain("play_example.com").is_err());
        assert!(validate_domain(&format!("{}.com", "a".repeat(64))).is_err());
    }

    #[test]
    fn rejects_unsafe_s3_prefixes_and_missing_credentials() {
        assert!(
            validate_s3_config(&S3Config {
                bucket: String::new(),
                region: None,
                endpoint_url: None,
                prefix: "backups".to_string(),
                access_key_env: "AWS_ACCESS_KEY_ID".to_string(),
                secret_key_env: "AWS_SECRET_ACCESS_KEY".to_string(),
            })
            .is_err()
        );
        assert!(
            validate_s3_config(&S3Config {
                bucket: "hostable".to_string(),
                region: None,
                endpoint_url: None,
                prefix: "../backups".to_string(),
                access_key_env: "AWS_ACCESS_KEY_ID".to_string(),
                secret_key_env: "AWS_SECRET_ACCESS_KEY".to_string(),
            })
            .is_err()
        );
        assert!(
            validate_s3_config(&S3Config {
                bucket: "hostable".to_string(),
                region: Some("us-east-1".to_string()),
                endpoint_url: None,
                prefix: "backups".to_string(),
                access_key_env: "AWS_ACCESS_KEY_ID".to_string(),
                secret_key_env: "AWS_SECRET_ACCESS_KEY".to_string(),
            })
            .is_ok()
        );
    }

    #[test]
    fn validates_backup_retention_count() {
        assert!(
            validate_backup_settings(&settings::BackupSettings {
                local_keep_last: 1,
                s3_keep_last: 1,
            })
            .is_ok()
        );
        assert!(
            validate_backup_settings(&settings::BackupSettings {
                local_keep_last: 100,
                s3_keep_last: 100,
            })
            .is_ok()
        );
        assert!(
            validate_backup_settings(&settings::BackupSettings {
                local_keep_last: 0,
                s3_keep_last: 1,
            })
            .is_err()
        );
        assert!(
            validate_backup_settings(&settings::BackupSettings {
                local_keep_last: 1,
                s3_keep_last: 101,
            })
            .is_err()
        );
    }
}
