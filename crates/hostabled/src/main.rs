mod auth;
mod backup;
mod executor;
mod minecraft;
mod s3;
mod settings;
mod system;
mod validate;

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use clap::Parser;
use hostable_core::{
    ActionPlan, ConfigDocument, ConfigPatch, CreateServerInput, GameAdapter, GameKind,
    ModrinthInstallFile, ModrinthInstallSelection, ModrinthProjectType, ModrinthSearchResponse,
    ModrinthVersion, RegistryStore, S3Config, ServerId, ServerRecord, adapter_for,
};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Component, PathBuf},
};
use tokio::net::TcpListener;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "hostabled", version)]
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
    public_port: u16,
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
        public_port: args.bind.port(),
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
        .route("/api/servers", get(list_servers))
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
        .route("/api/servers/{id}/overview", get(minecraft::overview))
        .route("/api/servers/{id}/players", get(minecraft::players))
        .route(
            "/api/servers/{id}/players/whitelist",
            post(minecraft::whitelist),
        )
        .route("/api/servers/{id}/players/ops", post(minecraft::ops))
        .route("/api/servers/{id}/mods", get(minecraft::list_mods))
        .route(
            "/api/servers/{id}/mods/install",
            post(minecraft::install_mod),
        )
        .route("/api/servers/{id}/mods/toggle", post(minecraft::toggle_mod))
        .route("/api/servers/{id}/mods/remove", post(minecraft::remove_mod))
        .route("/api/servers/{id}/backups/run", post(run_backup))
        .route("/api/settings", get(get_settings))
        .route("/api/settings/backups", put(update_backup_settings))
        .route("/api/settings/s3", put(update_s3_settings))
        .route("/api/settings/s3/test", post(test_s3_settings))
        .route("/api/settings/admin-web", put(update_admin_web_settings))
        .route("/api/admin/password", put(admin_change_password))
        .route("/api/network/public-ip", get(public_ip))
        .route("/api/preflight", get(preflight))
        .route("/api/system/disk", get(disk_report))
        .route("/api/system/storage", get(storage_report))
        .route("/api/system/disk/clean", post(disk_clean))
        .route("/api/system/swap", post(create_swap))
        .route("/api/servers/{id}/update", post(update_server))
        .route("/api/servers/{id}/doctor", get(server_doctor))
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
    .layer(middleware::from_fn_with_state(state.clone(), network_gate))
    .with_state(state.clone());

    let listener = TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("failed to bind daemon HTTP listener on {}", args.bind))?;

    println!("hostabled listening on http://{}", args.bind);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
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

/// Anything arriving from a non-loopback address is only allowed when the
/// admin web is enabled, and then only with a session cookie. The operator
/// token (full control) and first-time password setup are loopback-only, so a
/// public listener can never be used to take over the daemon.
async fn network_gate(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    if peer.ip().to_canonical().is_loopback() {
        return next.run(request).await;
    }
    let admin_enabled = state
        .settings
        .load()
        .map(|settings| settings.admin_web.enabled)
        .unwrap_or(false);
    let forbidden = |message: &'static str| {
        (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": message })),
        )
            .into_response()
    };
    if !admin_enabled {
        return forbidden("admin web is disabled");
    }
    if request.headers().contains_key(auth::OPERATOR_TOKEN_HEADER) {
        return forbidden("operator access is only available from the VPS itself");
    }
    if request.uri().path() == "/api/admin/setup" {
        return forbidden("set the admin password from the hostable TUI on the VPS");
    }
    next.run(request).await
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "hostabled",
    })
}

async fn adapters() -> Json<Vec<AdapterSummary>> {
    Json(
        hostable_core::ALL_GAMES
            .iter()
            .map(|kind| adapter_summary(hostable_core::adapter_for(*kind)))
            .collect(),
    )
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
    headers: HeaderMap,
    Json(request): Json<PasswordRequest>,
) -> Result<Response, ApiError> {
    if !state.auth.operator_authenticated_headers(&headers) {
        return Err(ApiError::Forbidden(
            "set the admin password from the hostable TUI on the VPS",
        ));
    }
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

    if state.auth.login_locked() {
        return Err(ApiError::TooManyRequests);
    }
    let ok = state.auth.verify_password(&request.password)?;
    state.auth.record_login_result(ok);
    if !ok {
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
        .header(
            "User-Agent",
            concat!("hostable/", env!("CARGO_PKG_VERSION")),
        )
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
        .header(
            "User-Agent",
            concat!("hostable/", env!("CARGO_PKG_VERSION")),
        );

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
    let version = select_modrinth_version(
        &client,
        &id,
        query.game_version.as_deref(),
        query.loader.as_deref(),
    )
    .await?;
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
                    query.game_version.as_deref(),
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
        game_versions: version.game_versions,
        loaders: version.loaders,
        file,
        dependencies,
    }))
}

async fn select_modrinth_version(
    client: &reqwest::Client,
    project_id: &str,
    game_version: Option<&str>,
    loader: Option<&str>,
) -> Result<ModrinthVersion, ApiError> {
    let loaders = loader.map(|loader| format!(r#"["{loader}"]"#));
    let mut request = client
        .get(format!(
            "https://api.modrinth.com/v2/project/{project_id}/version"
        ))
        .header(
            "User-Agent",
            concat!("hostable/", env!("CARGO_PKG_VERSION")),
        );
    if let Some(game_version) = game_version {
        let game_versions = format!(r#"["{game_version}"]"#);
        request = request.query(&[("game_versions", game_versions.as_str())]);
    }
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
        .header(
            "User-Agent",
            concat!("hostable/", env!("CARGO_PKG_VERSION")),
        )
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

async fn plan_server(
    Json(request): Json<CreateServerRequest>,
) -> Result<Json<ActionPlan>, ApiError> {
    validate::validate_create_input(&request.input, std::path::Path::new(validate::SERVERS_ROOT))
        .map_err(ApiError::BadRequest)?;
    let input = prepare_create_input(request.game, request.input);
    let plan = adapter_for(request.game).plan_install(input)?;
    Ok(Json(plan))
}

async fn create_server(
    State(state): State<AppState>,
    Json(request): Json<CreateServerRequest>,
) -> Result<Json<CreateServerResponse>, ApiError> {
    let game = request.game;
    validate::validate_create_input(&request.input, std::path::Path::new(validate::SERVERS_ROOT))
        .map_err(ApiError::BadRequest)?;
    let dry_run = request.dry_run;
    if !dry_run
        && state
            .registry
            .get_server(&ServerId(slugify(&request.input.name)))?
            .is_some()
    {
        return Err(ApiError::Conflict("a server with this name already exists"));
    }
    let input = assign_ports(&state, game, prepare_create_input(game, request.input))?;

    let report = system::preflight(
        &adapter_for(game).requirements(),
        input.memory_mb,
        std::path::Path::new(validate::SERVERS_ROOT),
        &running_servers(&state)?,
    );
    if !report.blockers.is_empty() && !dry_run {
        return Err(ApiError::Detail(
            StatusCode::INSUFFICIENT_STORAGE,
            report.blockers.join(" "),
        ));
    }

    let plan = adapter_for(game).plan_install(input.clone())?;
    let actions = plan.actions.clone();
    let mut log =
        blocking(move || executor::ActionExecutor::new(dry_run).execute(&actions)).await?;
    let server = server_record_from_input(game, &input);
    if !dry_run {
        state.registry.upsert_server(&server)?;
    }
    let mut warnings = report.warnings;
    warnings.extend(report.blockers);
    log.extend(warnings.iter().map(|warning| format!("warning: {warning}")));
    Ok(Json(CreateServerResponse {
        server,
        plan,
        log,
        warnings,
    }))
}

/// Pick game and RCON ports that no other server (or process) is using, so a second
/// server of the same game does not collide with the first.
fn assign_ports(
    state: &AppState,
    game: GameKind,
    mut input: CreateServerInput,
) -> Result<CreateServerInput, ApiError> {
    let adapter = adapter_for(game);
    let Some(main_port) = adapter.default_ports().first().cloned() else {
        return Ok(input);
    };
    // ARK uses a fixed set of ports and only one instance per host is supported.
    if game == GameKind::ArkSurvivalEvolved {
        return Ok(input);
    }
    let mut claimed: Vec<u16> = Vec::new();
    for server in state.registry.list_servers()? {
        claimed.extend(server.ports.iter().map(|port| port.port));
        if let Some(port) = server
            .adapter_config
            .pointer("/extra/rcon_port")
            .and_then(|value| value.as_u64())
        {
            claimed.push(port as u16);
        }
    }
    let mut extra = input.extra.as_object().cloned().unwrap_or_default();
    if !extra.contains_key("port") {
        let port = system::next_free_port(main_port.port, main_port.protocol, &claimed);
        claimed.push(port);
        extra.insert("port".to_string(), port.into());
    }
    if !extra.contains_key("rcon_port") {
        let rcon = system::next_free_port(25575, hostable_core::Protocol::Tcp, &claimed);
        extra.insert("rcon_port".to_string(), rcon.into());
    }
    input.extra = serde_json::Value::Object(extra);
    Ok(input)
}

/// Servers that are currently running, with the memory they hold.
fn running_servers(state: &AppState) -> Result<Vec<system::RunningServer>, ApiError> {
    let mut running = Vec::new();
    for server in state.registry.list_servers()? {
        if !service_is_active(&server) {
            continue;
        }
        let configured = server
            .adapter_config
            .get("memory_mb")
            .and_then(|value| value.as_u64());
        let reserved_mb = configured.unwrap_or_else(|| {
            u64::from(
                adapter_for(server.game)
                    .requirements()
                    .recommended_memory_mb,
            )
        });
        running.push(system::RunningServer {
            name: server.name,
            reserved_mb,
        });
    }
    Ok(running)
}

async fn import_server(
    State(state): State<AppState>,
    Json(request): Json<ImportServerRequest>,
) -> Result<Json<CreateServerResponse>, ApiError> {
    validate::validate_name(&request.name).map_err(ApiError::BadRequest)?;
    let install_path =
        validate::validate_import_path(&request.install_path).map_err(ApiError::BadRequest)?;
    if !request.dry_run
        && state
            .registry
            .get_server(&ServerId(slugify(&request.name)))?
            .is_some()
    {
        return Err(ApiError::Conflict("a server with this name already exists"));
    }
    let detected = adapter_for(request.game).detect_existing(&install_path)?;
    let plan = adapter_for(request.game).plan_import(detected.clone(), &request.name)?;
    let dry_run = request.dry_run;
    let actions = plan.actions.clone();
    let log = blocking(move || executor::ActionExecutor::new(dry_run).execute(&actions)).await?;
    let server = server_record_from_import(request.game, &request.name, detected);
    let mut warnings = Vec::new();
    let conflicts = system::conflicting_ports(&server.ports);
    if !conflicts.is_empty() && !service_is_active(&server) {
        warnings.push(format!(
            "port(s) {} are in use by another service right now. Hostable did not enable or start this server; stop the other one before starting it here.",
            conflicts.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
        ));
    }
    if request.game == GameKind::Palworld {
        warnings.extend(hostable_core::palworld::diagnose(&server.install_path));
    }
    if !dry_run {
        state.registry.upsert_server(&server)?;
    }
    Ok(Json(CreateServerResponse {
        server,
        plan,
        log,
        warnings,
    }))
}

fn prepare_create_input(game: GameKind, mut input: CreateServerInput) -> CreateServerInput {
    let mut extra = input
        .extra
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);
    let (key, prefix) = match game {
        GameKind::Minecraft => ("rcon_password", "rcon"),
        GameKind::ArkSurvivalEvolved | GameKind::Palworld => ("admin_password", "admin"),
    };
    extra
        .entry(key.to_string())
        .or_insert_with(|| serde_json::Value::String(format!("{prefix}-{}", Uuid::new_v4())));
    input.extra = serde_json::Value::Object(extra);
    input
}

async fn server_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ServiceStatusResponse>, ApiError> {
    let server = load_server(&state, id)?;
    let active = executor::systemctl_is_active(&server.service_name)?;
    Ok(Json(ServiceStatusResponse { active }))
}

async fn server_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<LogsResponse>, ApiError> {
    let server = load_server(&state, id)?;
    let lines = query.lines.unwrap_or(120).clamp(1, 500);
    let service = server.service_name.clone();
    let after = query.after;
    let (text, cursor) =
        blocking(move || executor::journalctl_tail(&service, lines, after.as_deref())).await?;
    Ok(Json(LogsResponse {
        service: server.service_name,
        lines,
        text: hostable_core::minecraft_admin::sanitize_text(&text),
        cursor,
    }))
}

async fn read_server_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ConfigDocument>, ApiError> {
    let server = load_server(&state, id)?;
    let config = adapter_for(server.game).read_config(&server)?;
    Ok(Json(config))
}

async fn write_server_config(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(patch): Json<ConfigPatch>,
) -> Result<Json<ConfigDocument>, ApiError> {
    let server = load_server(&state, id)?;
    adapter_for(server.game).write_config(&server, patch)?;
    read_server_config(State(state), Path(server.id.0)).await
}

#[derive(Debug, Deserialize, Default)]
struct ForceQuery {
    #[serde(default)]
    force: bool,
}

fn service_is_active(server: &ServerRecord) -> bool {
    executor::systemctl_is_active(&server.service_name)
        .map(|state| state == "active")
        .unwrap_or(false)
}

/// Refuse to interrupt players unless `force` is set. Returns warnings for the caller.
fn guard_players(server: &ServerRecord, force: bool) -> Result<Vec<String>, ApiError> {
    if !service_is_active(server) {
        return Ok(Vec::new());
    }
    let Some(console) = adapter_for(server.game).console() else {
        return Ok(Vec::new());
    };
    match console.online_players(server) {
        Ok(Some(0)) => Ok(Vec::new()),
        Ok(Some(count)) if !force => Err(ApiError::PlayersOnline(count)),
        Ok(Some(count)) => Ok(vec![format!("forced while {count} player(s) were online")]),
        Ok(None) => Ok(vec![
            "could not tell whether players are online".to_string(),
        ]),
        Err(error) => Ok(vec![format!(
            "could not check for players ({error}); the server may still be starting or RCON is off"
        )]),
    }
}

/// Ask the game to write its world to disk before the process is stopped.
fn save_before_stop(server: &ServerRecord) -> Vec<String> {
    if !service_is_active(server) {
        return Vec::new();
    }
    let Some(console) = adapter_for(server.game).console() else {
        return Vec::new();
    };
    match console.send_command(server, console.save_command()) {
        Ok(_) => Vec::new(),
        Err(error) => vec![format!("could not save before stopping: {error}")],
    }
}

async fn start_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ServiceActionResponse>, ApiError> {
    let server = load_server(&state, id)?;
    if !service_is_active(&server) {
        let conflicts = system::conflicting_ports(&server.ports);
        if !conflicts.is_empty() {
            return Err(ApiError::Detail(
                StatusCode::CONFLICT,
                format!(
                    "port {} is already in use by another process. Stop the other server first (for example `systemctl stop <service>`), then start this one.",
                    conflicts
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
    }
    executor::systemctl(&server.service_name, "start")?;
    Ok(Json(ServiceActionResponse::ok("start", Vec::new())))
}

async fn stop_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ForceQuery>,
) -> Result<Json<ServiceActionResponse>, ApiError> {
    let server = load_server(&state, id)?;
    let mut warnings = guard_players(&server, query.force)?;
    warnings.extend(
        blocking(move || {
            let warnings = save_before_stop(&server);
            executor::systemctl(&server.service_name, "stop")?;
            Ok(warnings)
        })
        .await?,
    );
    Ok(Json(ServiceActionResponse::ok("stop", warnings)))
}

async fn restart_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ForceQuery>,
) -> Result<Json<ServiceActionResponse>, ApiError> {
    let server = load_server(&state, id)?;
    let mut warnings = guard_players(&server, query.force)?;
    warnings.extend(
        blocking(move || {
            let warnings = save_before_stop(&server);
            executor::systemctl(&server.service_name, "restart")?;
            Ok(warnings)
        })
        .await?,
    );
    Ok(Json(ServiceActionResponse::ok("restart", warnings)))
}

/// Run slow, blocking work (apt, steamcmd, RCON, tar) off the async workers.
async fn blocking<T, F>(work: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ApiError::Internal(anyhow::anyhow!("background task failed: {error}")))?
        .map_err(ApiError::Internal)
}

async fn send_console_command(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ConsoleCommandRequest>,
) -> Result<Json<hostable_core::adapter::ConsoleCommandResult>, ApiError> {
    let server = load_server(&state, id)?;
    let command = request.command.trim().to_string();
    if command.is_empty() || command.len() > 512 || command.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(
            "console commands must be one line of at most 512 characters",
        ));
    }
    if adapter_for(server.game).console().is_none() {
        return Err(ApiError::BadRequest("this game has no console"));
    }
    let result = blocking(move || {
        let console = adapter_for(server.game)
            .console()
            .context("this game has no console")?;
        Ok(console.send_command(&server, &command)?)
    })
    .await?;
    Ok(Json(hostable_core::adapter::ConsoleCommandResult {
        accepted: result.accepted,
        message: hostable_core::minecraft_admin::sanitize_text(&result.message),
    }))
}

async fn run_backup(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<backup::BackupRunResult>, ApiError> {
    let server = load_server(&state, id)?;
    let settings = state.settings.load()?;
    let runner = backup::BackupRunner::new(state.backup_root.clone());
    let keep_last = settings.backup.local_keep_last;
    let backup_server = server.clone();
    let mut result = blocking(move || runner.run_with_retention(&backup_server, keep_last)).await?;

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
    let enabled = config.enabled;
    settings.admin_web = config;
    state.settings.save(&settings)?;
    // The daemon listens on 0.0.0.0 but the firewall decides who can reach it.
    if let Err(error) = executor::ufw_set_port(state.public_port, enabled) {
        eprintln!("failed to update firewall for the admin web: {error:#}");
    }
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
        .header(
            "User-Agent",
            concat!("hostable/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(Json(PublicIpResponse {
        ip: ip.trim().to_string(),
    }))
}

#[derive(Debug, Deserialize)]
struct PreflightQuery {
    game: GameKind,
    memory_mb: Option<u32>,
}

async fn preflight(
    State(state): State<AppState>,
    Query(query): Query<PreflightQuery>,
) -> Result<Json<system::Preflight>, ApiError> {
    Ok(Json(system::preflight(
        &adapter_for(query.game).requirements(),
        query.memory_mb,
        std::path::Path::new(validate::SERVERS_ROOT),
        &running_servers(&state)?,
    )))
}

async fn disk_report(State(state): State<AppState>) -> Result<Json<system::DiskReport>, ApiError> {
    let registered: Vec<PathBuf> = state
        .registry
        .list_servers()?
        .into_iter()
        .map(|server| server.install_path)
        .collect();
    let backups_root = state.backup_root.clone();
    let report = blocking(move || {
        let extra_roots: Vec<PathBuf> = std::fs::read_dir("/home")
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default();
        Ok(system::disk_report(&system::DiskScanInput {
            servers_root: std::path::Path::new(validate::SERVERS_ROOT),
            backups_root: &backups_root,
            registered_paths: &registered,
            extra_roots: &extra_roots,
        }))
    })
    .await?;
    Ok(Json(report))
}

async fn storage_report(
    State(state): State<AppState>,
) -> Result<Json<system::StorageReport>, ApiError> {
    let servers: Vec<(String, PathBuf)> = state
        .registry
        .list_servers()?
        .into_iter()
        .map(|server| (server.id.0, server.install_path))
        .collect();
    let report = blocking(move || {
        Ok(system::storage_report(
            &servers,
            std::path::Path::new(validate::SERVERS_ROOT),
        ))
    })
    .await?;
    Ok(Json(report))
}

#[derive(Debug, Deserialize)]
struct DiskCleanRequest {
    action: String,
}

async fn disk_clean(
    Json(request): Json<DiskCleanRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let message = blocking(move || system::disk_clean(&request.action)).await?;
    Ok(Json(serde_json::json!({ "ok": true, "message": message })))
}

#[derive(Debug, Deserialize)]
struct SwapRequest {
    size_mb: u32,
}

async fn create_swap(
    Json(request): Json<SwapRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let message = blocking(move || system::ensure_swap(request.size_mb)).await?;
    Ok(Json(serde_json::json!({ "ok": true, "message": message })))
}

#[derive(Debug, Serialize)]
struct UpdateResponse {
    ok: bool,
    backup: Option<PathBuf>,
    build_before: Option<String>,
    build_after: Option<String>,
    log: Vec<String>,
    warnings: Vec<String>,
}

/// Update the game in place: stop (never over players), back up, update, start again.
async fn update_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ForceQuery>,
) -> Result<Json<UpdateResponse>, ApiError> {
    let server = load_server(&state, id)?;
    let adapter = adapter_for(server.game);
    let Some(actions) = adapter.plan_update(&server) else {
        return Err(ApiError::BadRequest(
            "this game cannot be updated in place yet; for modpacks, install the new version next to the old one and copy the world over",
        ));
    };
    let mut warnings = guard_players(&server, query.force)?;
    let build_before = adapter.installed_build(&server);
    let was_active = service_is_active(&server);

    // Back up first. If it fails, stop here: never update without a way back.
    let keep_last = state.settings.load()?.backup.local_keep_last;
    let runner = backup::BackupRunner::new(state.backup_root.clone());
    let backup_server = server.clone();
    let backup = blocking(move || runner.run_with_retention(&backup_server, keep_last))
        .await
        .map_err(|error| match error {
            ApiError::Internal(inner) => ApiError::Detail(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("backup failed, so nothing was changed: {inner}"),
            ),
            other => other,
        })?;
    warnings.extend(backup.warnings.clone());

    let service = server.service_name.clone();
    let job_server = server.clone();
    let outcome = blocking(move || {
        if was_active {
            save_before_stop(&job_server);
            executor::systemctl(&service, "stop")?;
        }
        let result = executor::ActionExecutor::new(false).execute(&actions);
        // Start again even if the update failed, so a bad update does not leave the
        // server down (the backup is the way back).
        if was_active {
            executor::systemctl(&service, "start")?;
        }
        result
    })
    .await;

    let build_after = adapter.installed_build(&server);
    match outcome {
        Ok(log) => Ok(Json(UpdateResponse {
            ok: true,
            backup: Some(backup.archive_path),
            build_before,
            build_after,
            log,
            warnings,
        })),
        Err(ApiError::Internal(error)) => Err(ApiError::Detail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "update failed: {error}. A backup was taken first: {}",
                backup.archive_path.display()
            ),
        )),
        Err(other) => Err(other),
    }
}

async fn server_doctor(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<String>>, ApiError> {
    let server = load_server(&state, id)?;
    let mut findings = Vec::new();
    if server.game == GameKind::Palworld {
        findings.extend(hostable_core::palworld::diagnose(&server.install_path));
    }
    if service_is_active(&server) {
        if let Some(console) = adapter_for(server.game).console()
            && let Err(error) = console.online_players(&server)
        {
            findings.push(format!(
                "RCON is not answering ({error}). Console, safe restarts and pre-backup saves need it."
            ));
        }
    } else {
        let conflicts = system::conflicting_ports(&server.ports);
        if !conflicts.is_empty() {
            findings.push(format!(
                "port(s) {} are used by another process, so this server cannot start.",
                conflicts
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    Ok(Json(findings))
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
    /// Left out for modpacks: the pack decides its own Minecraft version.
    game_version: Option<String>,
    loader: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    lines: Option<u16>,
    /// Cursor from an earlier response: return only newer lines.
    after: Option<String>,
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
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ServiceStatusResponse {
    active: String,
}

#[derive(Debug, Serialize)]
struct ServiceActionResponse {
    ok: bool,
    action: &'static str,
    warnings: Vec<String>,
}

impl ServiceActionResponse {
    fn ok(action: &'static str, warnings: Vec<String>) -> Self {
        Self {
            ok: true,
            action,
            warnings,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct LogsResponse {
    service: String,
    lines: u16,
    text: String,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct PublicIpResponse {
    ip: String,
}

fn server_record_from_input(game: GameKind, input: &CreateServerInput) -> ServerRecord {
    let id = slugify(&input.name);
    let mut ports = adapter_for(game).default_ports();
    if let Some(port) = input
        .extra
        .get("port")
        .and_then(|value| value.as_u64())
        .and_then(|value| u16::try_from(value).ok())
        && let Some(first) = ports.first_mut()
    {
        first.port = port;
    }
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
    let ports = if detected.ports.is_empty() {
        adapter_for(game).default_ports()
    } else {
        detected.ports.clone()
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
            "extra": detected.extra,
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

fn load_server(state: &AppState, id: String) -> Result<ServerRecord, ApiError> {
    let server = state
        .registry
        .get_server(&ServerId(id))?
        .ok_or(ApiError::NotFound)?;
    if !validate::is_safe_service_name(&server.service_name) {
        return Err(ApiError::Forbidden(
            "server record has an unmanaged service name",
        ));
    }
    Ok(server)
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
    TooManyRequests,
    /// An error with a message built at runtime.
    Detail(StatusCode, String),
    /// The action would interrupt players. Clients can retry with `force=true`.
    PlayersOnline(u32),
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
            ApiError::Detail(status, message) => {
                (status, Json(serde_json::json!({ "error": message }))).into_response()
            }
            ApiError::PlayersOnline(count) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": format!(
                        "{count} player(s) are online. Ask them to leave, or repeat with force=true to interrupt them."
                    ),
                    "code": "players_online",
                    "players": count,
                })),
            )
                .into_response(),
            ApiError::TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({ "error": "too many attempts, try again in a minute" })),
            )
                .into_response(),
            ApiError::Internal(error) => {
                eprintln!("internal error: {error:#}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": error.to_string() })),
                )
                    .into_response()
            }
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
