//! Read-only overview plus day-two management (mods, players) for a server.
//! Mods and the whitelist are Minecraft-only; the overview works for every game.

use crate::{ApiError, AppState, blocking, load_server};
use anyhow::Context;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use hostable_core::{
    GameKind, ModrinthProjectType, ServerRecord, adapter_for,
    minecraft_admin::{self as admin, InstalledMod, InstalledRecord, OnlinePlayers, PlayerEntry},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::{
    io::Write,
    os::unix::fs::{MetadataExt, chown},
    path::{Path as FsPath, PathBuf},
};

fn minecraft_only(server: &ServerRecord) -> Result<(), ApiError> {
    if server.game == GameKind::Minecraft {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "this is only available for Minecraft servers",
        ))
    }
}

fn server_type(server: &ServerRecord) -> String {
    let recorded = server
        .adapter_config
        .get("server_type")
        .and_then(|value| value.as_str())
        .unwrap_or("vanilla");
    // Older imports recorded Fabric servers as vanilla; trust the folder over the record.
    if recorded == "vanilla" && admin::has_fabric_launcher(&server.install_path) {
        return "fabric".to_string();
    }
    recorded.to_string()
}

fn rcon(server: &ServerRecord, command: &str) -> anyhow::Result<String> {
    let console = adapter_for(server.game)
        .console()
        .context("this game has no console")?;
    Ok(admin::sanitize_text(
        &console.send_command(server, command)?.message,
    ))
}

fn active(server: &ServerRecord) -> bool {
    crate::service_is_active(server)
}

#[derive(Debug, Serialize)]
pub struct Overview {
    pub active: bool,
    pub game: GameKind,
    pub server_type: String,
    pub game_version: Option<String>,
    pub players: Option<OnlinePlayers>,
    /// Why the player count is missing (server stopped, RCON off, ...).
    pub players_note: Option<String>,
    pub motd: Option<String>,
    pub max_players: Option<u32>,
    pub online_mode: Option<bool>,
    pub whitelist_enabled: Option<bool>,
    pub whitelist_count: Option<usize>,
    pub ops_count: Option<usize>,
    pub mods_count: Option<usize>,
    pub mods_kind: Option<&'static str>,
}

pub async fn overview(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Overview>, ApiError> {
    let server = load_server(&state, id)?;
    Ok(Json(blocking(move || Ok(build_overview(&server))).await?))
}

fn build_overview(server: &ServerRecord) -> Overview {
    let is_active = active(server);
    let kind = server_type(server);
    let mut overview = Overview {
        active: is_active,
        game: server.game,
        server_type: kind.clone(),
        game_version: None,
        players: None,
        players_note: None,
        motd: None,
        max_players: None,
        online_mode: None,
        whitelist_enabled: None,
        whitelist_count: None,
        ops_count: None,
        mods_count: None,
        mods_kind: None,
    };

    if !is_active {
        overview.players_note = Some("server is stopped".to_string());
    } else if let Some(console) = adapter_for(server.game).console() {
        match console.online_players(server) {
            Ok(Some(online)) => {
                overview.players = Some(OnlinePlayers {
                    online,
                    max: None,
                    names: Vec::new(),
                });
            }
            Ok(None) => overview.players_note = Some("player count unavailable".to_string()),
            Err(_) => {
                overview.players_note =
                    Some("console (RCON) is not answering, so players are unknown".to_string())
            }
        }
    }

    if server.game != GameKind::Minecraft {
        return overview;
    }

    let props = admin::read_properties(&server.install_path);
    overview.motd = props.motd;
    overview.max_players = props.max_players;
    overview.online_mode = Some(props.online_mode);
    overview.whitelist_enabled = Some(props.white_list);
    overview.whitelist_count =
        Some(admin::read_player_file(&server.install_path.join("whitelist.json")).len());
    overview.ops_count = Some(admin::read_player_file(&server.install_path.join("ops.json")).len());
    overview.game_version = admin::detect_game_version(
        &server.install_path,
        server
            .adapter_config
            .get("game_version")
            .and_then(|value| value.as_str()),
    );
    if is_active
        && overview.players.is_some()
        && let Ok(reply) = rcon(server, "list")
        && let Some(parsed) = admin::parse_list_reply(&reply)
    {
        overview.players = Some(parsed);
    }
    if let Some(dir) = admin::extensions_dir(&server.install_path, &kind) {
        overview.mods_kind = Some(admin::extension_noun(&kind));
        overview.mods_count = Some(
            admin::list_mods(&server.install_path, &dir)
                .map(|mods| mods.iter().filter(|item| item.enabled).count())
                .unwrap_or(0),
        );
    }
    overview
}

#[derive(Debug, Serialize)]
pub struct PlayersResponse {
    pub online: Option<OnlinePlayers>,
    pub whitelist_enabled: bool,
    pub whitelist: Vec<PlayerEntry>,
    pub ops: Vec<PlayerEntry>,
    pub banned: Vec<PlayerEntry>,
    pub online_mode: bool,
}

pub async fn players(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PlayersResponse>, ApiError> {
    let server = load_server(&state, id)?;
    minecraft_only(&server)?;
    let response = blocking(move || {
        let props = admin::read_properties(&server.install_path);
        let online = if active(&server) {
            rcon(&server, "list")
                .ok()
                .and_then(|reply| admin::parse_list_reply(&reply))
        } else {
            None
        };
        let file = |name: &str| admin::read_player_file(&server.install_path.join(name));
        Ok(PlayersResponse {
            online,
            whitelist_enabled: props.white_list,
            whitelist: file("whitelist.json"),
            ops: file("ops.json"),
            banned: file("banned-players.json"),
            online_mode: props.online_mode,
        })
    })
    .await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct ListAction {
    /// whitelist: add | remove | on | off. ops: op | deop.
    pub action: String,
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ActionReply {
    pub message: String,
}

fn name_arg(request: &ListAction) -> Result<&str, ApiError> {
    let name = request.name.as_deref().unwrap_or_default();
    if !admin::valid_player_name(name) {
        return Err(ApiError::BadRequest(
            "player names are up to 16 letters, digits or underscores",
        ));
    }
    Ok(name)
}

pub async fn whitelist(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ListAction>,
) -> Result<Json<ActionReply>, ApiError> {
    let server = load_server(&state, id)?;
    minecraft_only(&server)?;
    let command = match request.action.as_str() {
        "add" => format!("whitelist add {}", name_arg(&request)?),
        "remove" => format!("whitelist remove {}", name_arg(&request)?),
        "on" => "whitelist on".to_string(),
        "off" => "whitelist off".to_string(),
        _ => return Err(ApiError::BadRequest("unknown whitelist action")),
    };
    run_rcon(server, command).await
}

pub async fn ops(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ListAction>,
) -> Result<Json<ActionReply>, ApiError> {
    let server = load_server(&state, id)?;
    minecraft_only(&server)?;
    let command = match request.action.as_str() {
        "op" => format!("op {}", name_arg(&request)?),
        "deop" => format!("deop {}", name_arg(&request)?),
        _ => return Err(ApiError::BadRequest("unknown ops action")),
    };
    run_rcon(server, command).await
}

async fn run_rcon(server: ServerRecord, command: String) -> Result<Json<ActionReply>, ApiError> {
    if !active(&server) {
        return Err(ApiError::Conflict(
            "start the server first: whitelist and operator changes go through its console",
        ));
    }
    let message = blocking(move || rcon(&server, &command)).await?;
    Ok(Json(ActionReply { message }))
}

#[derive(Debug, Serialize)]
pub struct ModsResponse {
    /// "mod" (Fabric), "plugin" (Paper) or null when the server type cannot load either.
    pub kind: Option<&'static str>,
    pub server_type: String,
    pub game_version: Option<String>,
    pub loader: Option<&'static str>,
    pub items: Vec<InstalledMod>,
}

pub async fn list_mods(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ModsResponse>, ApiError> {
    let server = load_server(&state, id)?;
    minecraft_only(&server)?;
    let response = blocking(move || {
        let kind = server_type(&server);
        let dir = admin::extensions_dir(&server.install_path, &kind);
        let items = match &dir {
            Some(dir) => admin::list_mods(&server.install_path, dir)?,
            None => Vec::new(),
        };
        Ok(ModsResponse {
            kind: dir.as_ref().map(|_| admin::extension_noun(&kind)),
            loader: loader_for(&kind),
            game_version: admin::detect_game_version(
                &server.install_path,
                server
                    .adapter_config
                    .get("game_version")
                    .and_then(|value| value.as_str()),
            ),
            server_type: kind,
            items,
        })
    })
    .await?;
    Ok(Json(response))
}

fn loader_for(server_type: &str) -> Option<&'static str> {
    match server_type {
        "fabric" => Some("fabric"),
        "paper" => Some("paper"),
        _ => None,
    }
}

fn mods_dir(server: &ServerRecord) -> Result<PathBuf, ApiError> {
    admin::extensions_dir(&server.install_path, &server_type(server)).ok_or(ApiError::BadRequest(
        "vanilla servers cannot load mods or plugins; use Fabric or Paper",
    ))
}

#[derive(Debug, Deserialize)]
pub struct InstallRequest {
    pub project_id: String,
    pub title: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InstallResponse {
    pub installed: Vec<String>,
    pub skipped: Vec<String>,
    pub restart_required: bool,
}

pub async fn install_mod(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<InstallRequest>,
) -> Result<Json<InstallResponse>, ApiError> {
    let server = load_server(&state, id)?;
    minecraft_only(&server)?;
    let dir = mods_dir(&server)?;
    if request.project_id.is_empty()
        || !request
            .project_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(ApiError::BadRequest("invalid Modrinth project id"));
    }
    let kind = server_type(&server);
    let game_version = admin::detect_game_version(
        &server.install_path,
        server
            .adapter_config
            .get("game_version")
            .and_then(|value| value.as_str()),
    )
    .ok_or(ApiError::BadRequest(
        "could not tell the Minecraft version; start the server once so it is written to the log",
    ))?;

    let selection = crate::modrinth_install_selection(
        Path(request.project_id.clone()),
        axum::extract::Query(crate::ModrinthInstallQuery {
            project_type: ModrinthProjectType::Mod,
            game_version: Some(game_version),
            loader: loader_for(&kind).map(str::to_string),
        }),
    )
    .await
    .map_err(|error| match error {
        ApiError::NotFound => ApiError::Detail(
            StatusCode::NOT_FOUND,
            "no compatible version of that project for this server's Minecraft version and loader"
                .to_string(),
        ),
        other => other,
    })?
    .0;

    let mut files = vec![(selection.file.clone(), true)];
    files.extend(
        selection
            .dependencies
            .iter()
            .cloned()
            .map(|file| (file, false)),
    );
    let mut installed = Vec::new();
    let mut skipped = Vec::new();
    for (file, is_main) in files {
        admin::check_mod_filename(&file.filename)?;
        let target_exists = dir.join(&file.filename).exists()
            || dir.join(format!("{}.disabled", file.filename)).exists();
        if target_exists {
            skipped.push(file.filename);
            continue;
        }
        download_to(&dir, &file).await?;
        if is_main {
            let record = InstalledRecord {
                project_id: selection.project_id.clone(),
                title: request
                    .title
                    .clone()
                    .map(|title| admin::sanitize_text(&title))
                    .unwrap_or_else(|| file.filename.clone()),
                version: selection.version_number.clone(),
            };
            admin::record_install(&server.install_path, &file.filename, record)?;
        }
        installed.push(file.filename);
    }
    Ok(Json(InstallResponse {
        installed,
        skipped,
        restart_required: true,
    }))
}

const MAX_MOD_BYTES: u64 = 256 * 1024 * 1024;

async fn download_to(
    dir: &FsPath,
    file: &hostable_core::ModrinthInstallFile,
) -> Result<(), ApiError> {
    let url =
        reqwest::Url::parse(&file.url).map_err(|_| ApiError::BadRequest("bad download url"))?;
    if url.scheme() != "https" || url.host_str() != Some("cdn.modrinth.com") {
        return Err(ApiError::BadRequest(
            "mods can only be downloaded from Modrinth",
        ));
    }
    if file.size > MAX_MOD_BYTES {
        return Err(ApiError::BadRequest(
            "that file is larger than the 256 MB limit",
        ));
    }
    tokio::fs::create_dir_all(dir).await?;
    let bytes = reqwest::Client::new()
        .get(url)
        .header(
            "User-Agent",
            concat!("hostable/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if bytes.len() as u64 > MAX_MOD_BYTES {
        return Err(ApiError::BadRequest(
            "that file is larger than the 256 MB limit",
        ));
    }
    if let Some(expected) = &file.sha512 {
        let digest = Sha512::digest(&bytes);
        let actual: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(ApiError::Detail(
                StatusCode::BAD_GATEWAY,
                format!(
                    "{} failed its checksum, nothing was installed",
                    file.filename
                ),
            ));
        }
    }

    let dir = dir.to_path_buf();
    let filename = file.filename.clone();
    blocking(move || {
        let part = dir.join(format!(".{filename}.part"));
        let mut handle = std::fs::File::create(&part)?;
        handle.write_all(&bytes)?;
        handle.sync_all()?;
        // Keep the server user as the owner so the service can read and delete it.
        let meta = std::fs::metadata(&dir)?;
        let _ = chown(&part, Some(meta.uid()), Some(meta.gid()));
        std::fs::rename(&part, dir.join(&filename))?;
        Ok(())
    })
    .await
}

#[derive(Debug, Deserialize)]
pub struct ModFileRequest {
    pub filename: String,
}

pub async fn toggle_mod(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ModFileRequest>,
) -> Result<Json<ActionReply>, ApiError> {
    let server = load_server(&state, id)?;
    minecraft_only(&server)?;
    let dir = mods_dir(&server)?;
    let enable = request.filename.ends_with(".disabled");
    let new_name = admin::set_mod_enabled(&dir, &request.filename, enable)?;
    Ok(Json(ActionReply {
        message: format!(
            "{new_name} is now {}. Restart the server to apply.",
            if enable { "enabled" } else { "disabled" }
        ),
    }))
}

pub async fn remove_mod(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ModFileRequest>,
) -> Result<Json<ActionReply>, ApiError> {
    let server = load_server(&state, id)?;
    minecraft_only(&server)?;
    let dir = mods_dir(&server)?;
    admin::remove_mod(&server.install_path, &dir, &request.filename)?;
    Ok(Json(ActionReply {
        message: format!("Removed {}. Restart the server to apply.", request.filename),
    }))
}
