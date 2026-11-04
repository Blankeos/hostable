use super::{STEAMCMD, owner_or_hostable, slugify};
use crate::actions::{ActionPlan, HostAction};
use crate::adapter::{
    ConfigDocument, ConfigFile, ConfigFormat, ConfigPatch, ConsoleCapability, CreateServerInput,
    DetectedServer, DetectionConfidence, GameAdapter, GameKind, HealthStatus, PortSpec, Protocol,
    Requirements, ServerState,
};
use crate::backup::{BackupHook, BackupSpec};
use crate::registry::ServerRecord;
use crate::{HostableError, Result};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

/// Steam app id of the Palworld dedicated server.
pub const PALWORLD_APP_ID: &str = "2394010";
const SETTINGS_RELATIVE: &str = "Pal/Saved/Config/LinuxServer/PalWorldSettings.ini";
const DEFAULT_SETTINGS_RELATIVE: &str = "DefaultPalWorldSettings.ini";
const SAVE_GAMES_RELATIVE: &str = "Pal/Saved/SaveGames/0";
const DEFAULT_RCON_PORT: u16 = 25575;

#[derive(Debug, Default)]
pub struct PalworldAdapter;

impl GameAdapter for PalworldAdapter {
    fn kind(&self) -> GameKind {
        GameKind::Palworld
    }

    fn display_name(&self) -> &'static str {
        "Palworld"
    }

    fn default_ports(&self) -> Vec<PortSpec> {
        vec![PortSpec {
            port: 8211,
            protocol: Protocol::Udp,
            label: "Game".to_string(),
        }]
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            packages: vec![
                "steamcmd".to_string(),
                "lib32gcc-s1".to_string(),
                "ufw".to_string(),
            ],
            architectures: vec!["i386".to_string()],
            apt_components: vec![
                "contrib".to_string(),
                "non-free".to_string(),
                "non-free-firmware".to_string(),
            ],
            // Pocketpair recommends 16 GB; 8 GB works with swap and few players.
            min_memory_mb: 8 * 1024,
            recommended_memory_mb: 16 * 1024,
            min_disk_mb: 12 * 1024,
        }
    }

    fn detect_existing(&self, path: &Path) -> Result<DetectedServer> {
        if !path.join("PalServer.sh").exists() {
            return Err(HostableError::InvalidInput(format!(
                "{} does not look like a Palworld server (PalServer.sh not found)",
                path.display()
            )));
        }
        Ok(DetectedServer {
            kind: GameKind::Palworld,
            install_path: path.to_path_buf(),
            server_type: Some(crate::ServerType::Palworld),
            confidence: DetectionConfidence::High,
            ports: self.default_ports(),
            extra: palworld_extra(path),
        })
    }

    fn plan_install(&self, input: CreateServerInput) -> Result<ActionPlan> {
        if !input.accepted_license {
            return Err(HostableError::InvalidInput(
                "Steam/SteamCMD license requirements must be accepted before installation"
                    .to_string(),
            ));
        }

        let reqs = self.requirements();
        let root = input.install_path.clone();
        let mut plan = ActionPlan::new(format!("Create Palworld server '{}'", input.name));
        plan.actions.push(HostAction::EnableAptComponent {
            components: reqs.apt_components,
        });
        plan.actions.push(HostAction::AddArchitecture {
            arch: "i386".to_string(),
        });
        plan.actions.push(HostAction::AptInstall {
            packages: reqs.packages,
        });
        plan.actions.push(HostAction::CreateUser {
            user: "hostable".to_string(),
            system: true,
        });
        plan.actions.push(HostAction::CreateDirectory {
            path: root.clone(),
            owner: "hostable".to_string(),
        });
        plan.actions.push(steamcmd_update(&root));
        plan.actions.push(HostAction::CreateDirectory {
            path: root.join("Pal/Saved/Config/LinuxServer"),
            owner: "hostable".to_string(),
        });
        // The game ships a complete default config but leaves the live one empty on
        // first boot, which silently runs every setting at its default. Seed it from
        // the default file and then apply our own options on top.
        plan.actions.push(HostAction::PatchPalworldSettings {
            path: root.join(SETTINGS_RELATIVE),
            seed_from: Some(root.join(DEFAULT_SETTINGS_RELATIVE)),
            options: settings_options(&input),
            owner: "hostable".to_string(),
        });
        plan.actions.push(HostAction::WriteFile {
            path: root.join("start.sh"),
            contents: render_start_script(&input),
            owner: "hostable".to_string(),
            mode: 0o755,
        });
        plan.actions.push(HostAction::UfwAllow {
            port: super::extra_port(&input, "port", 8211),
            protocol: Protocol::Udp,
        });
        let unit = format!("hostable-{}.service", slugify(&input.name));
        plan.actions.push(HostAction::RenderSystemdUnit {
            name: unit.clone(),
            contents: render_systemd_unit(&input),
        });
        plan.actions.push(HostAction::SystemdDaemonReload);
        plan.actions.push(HostAction::SystemdEnable { unit });
        Ok(plan)
    }

    fn plan_import(&self, detected: DetectedServer, name: &str) -> Result<ActionPlan> {
        let root = &detected.install_path;
        // The owner's own launcher wins; otherwise run the official script directly.
        let exec_start = match ["start.sh", "run.sh"]
            .iter()
            .map(|file| root.join(file))
            .find(|path| path.exists())
        {
            Some(script) => format!("/usr/bin/env bash {}", script.display()),
            None => format!(
                "{}/PalServer.sh -port=8211 -useperfthreads -NoAsyncLoadingThread -UseMultithreadForDS",
                root.display()
            ),
        };
        super::import_plan(
            "Import Palworld server",
            name,
            &format!("Hostable Palworld server {name}"),
            root,
            &exec_start,
            &detected.ports,
            180,
        )
    }

    fn read_config(&self, server: &ServerRecord) -> Result<ConfigDocument> {
        let path = server.install_path.join(SETTINGS_RELATIVE);
        let mut files = Vec::new();
        if path.exists() {
            files.push(ConfigFile {
                path: path.clone(),
                format: ConfigFormat::Ini,
                contents: fs::read_to_string(&path)?,
            });
        }
        Ok(ConfigDocument {
            title: format!("Palworld config for {}", server.name),
            files,
        })
    }

    fn write_config(&self, server: &ServerRecord, patch: ConfigPatch) -> Result<()> {
        for file in patch.files {
            let path = safe_config_path(&server.install_path, &file.path)?;
            fs::write(path, file.contents)?;
        }
        Ok(())
    }

    fn backup_spec(&self, server: &ServerRecord) -> BackupSpec {
        BackupSpec {
            include_paths: vec![server.install_path.join("Pal/Saved")],
            exclude_paths: vec![server.install_path.join("Pal/Saved/Logs")],
            pre_copy_hook: Some(BackupHook::RconSave),
        }
    }

    fn health_check(&self, _server: &ServerRecord) -> Result<HealthStatus> {
        Ok(HealthStatus {
            state: ServerState::Unknown,
            detail: "Use hostabled service status for the live systemd state".to_string(),
        })
    }

    fn console(&self) -> Option<&dyn ConsoleCapability> {
        Some(self)
    }

    fn plan_update(&self, server: &ServerRecord) -> Option<Vec<HostAction>> {
        Some(plan_update(&server.install_path))
    }

    fn installed_build(&self, server: &ServerRecord) -> Option<String> {
        read_build_id(&server.install_path)
    }
}

impl ConsoleCapability for PalworldAdapter {
    fn command_examples(&self) -> Vec<&'static str> {
        vec![
            "Info",
            "ShowPlayers",
            "Broadcast Restarting_in_5_minutes",
            "Save",
        ]
    }

    fn send_command(
        &self,
        server: &ServerRecord,
        command: &str,
    ) -> Result<crate::adapter::ConsoleCommandResult> {
        let password = server
            .adapter_config
            .pointer("/extra/admin_password")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                HostableError::InvalidInput("server has no Palworld admin password".to_string())
            })?;
        let port = server
            .adapter_config
            .pointer("/extra/rcon_port")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_RCON_PORT as u64);
        let message = crate::rcon::send_command(&format!("127.0.0.1:{port}"), password, command)?;
        Ok(crate::adapter::ConsoleCommandResult {
            accepted: true,
            message,
        })
    }

    fn save_command(&self) -> &'static str {
        "Save"
    }

    fn online_players(&self, server: &ServerRecord) -> Result<Option<u32>> {
        let reply = self.send_command(server, "ShowPlayers")?.message;
        Ok(Some(count_palworld_players(&reply)))
    }
}

/// `ShowPlayers` replies with a `name,playeruid,steamid` header and one CSV row per player.
fn count_palworld_players(reply: &str) -> u32 {
    reply
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && line.contains(','))
        .filter(|line| !line.starts_with("name,"))
        .count() as u32
}

/// Steam build id from `steamapps/appmanifest_<app>.acf`.
pub fn read_build_id(install_path: &Path) -> Option<String> {
    let manifest = fs::read_to_string(
        install_path.join(format!("steamapps/appmanifest_{PALWORLD_APP_ID}.acf")),
    )
    .ok()?;
    manifest.lines().find_map(|line| {
        let mut parts = line.split('"').filter(|part| !part.trim().is_empty());
        (parts.next()? == "buildid").then(|| parts.next().map(str::to_string))?
    })
}

/// RCON settings of an existing install, so the console and safe restarts work
/// without the user re-entering anything.
fn palworld_extra(install_path: &Path) -> serde_json::Value {
    let mut extra = serde_json::Map::new();
    if let Ok(contents) = fs::read_to_string(install_path.join(SETTINGS_RELATIVE))
        && let Some(pairs) = parse_option_settings(&contents)
    {
        let get = |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.trim_matches('"').to_string())
        };
        if get("RCONEnabled").as_deref() == Some("True")
            && let Some(password) = get("AdminPassword").filter(|value| !value.is_empty())
        {
            extra.insert("admin_password".to_string(), password.into());
            if let Some(port) = get("RCONPort").and_then(|value| value.parse::<u16>().ok()) {
                extra.insert("rcon_port".to_string(), port.into());
            }
        }
    }
    serde_json::Value::Object(extra)
}

/// Problems in an existing Palworld install that silently change behaviour.
pub fn diagnose(install_path: &Path) -> Vec<String> {
    let mut findings = Vec::new();
    let settings = install_path.join(SETTINGS_RELATIVE);
    match fs::read_to_string(&settings) {
        Ok(contents) if parse_option_settings(&contents).is_none() => findings.push(format!(
            "{} is empty, so the server is running with built-in defaults. Seed it from DefaultPalWorldSettings.ini.",
            settings.display()
        )),
        Err(_) => findings.push(format!("{} does not exist yet", settings.display())),
        _ => {}
    }
    if let Ok(worlds) = fs::read_dir(install_path.join(SAVE_GAMES_RELATIVE)) {
        for world in worlds.flatten() {
            let option_sav = world.path().join("WorldOption.sav");
            if option_sav.exists() {
                findings.push(format!(
                    "{} overrides PalWorldSettings.ini (common after a co-op import). Move it aside to make INI settings such as BaseCampWorkerMaxNum apply.",
                    option_sav.display()
                ));
            }
        }
    }
    findings
}

fn steamcmd_update(root: &Path) -> HostAction {
    HostAction::RunCommand {
        program: STEAMCMD.to_string(),
        args: vec![
            "+force_install_dir".to_string(),
            root.to_string_lossy().into_owned(),
            "+login".to_string(),
            "anonymous".to_string(),
            "+app_update".to_string(),
            PALWORLD_APP_ID.to_string(),
            "validate".to_string(),
            "+quit".to_string(),
        ],
        user: Some(owner_or_hostable(root)),
        cwd: Some(root.to_path_buf()),
    }
}

/// Steps to update an installed server. The caller stops the service and takes a
/// backup first, then restarts and checks the version afterwards.
pub fn plan_update(install_path: &Path) -> Vec<HostAction> {
    vec![steamcmd_update(install_path)]
}

fn extra_str<'a>(input: &'a CreateServerInput, key: &str) -> Option<&'a str> {
    input.extra.get(key).and_then(|value| value.as_str())
}

fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('"', ""))
}

fn settings_options(input: &CreateServerInput) -> Vec<(String, String)> {
    let max_players = input
        .extra
        .get("max_players")
        .and_then(|value| value.as_u64())
        .unwrap_or(8);
    let rcon_port = super::extra_port(input, "rcon_port", DEFAULT_RCON_PORT);
    let port = super::extra_port(input, "port", 8211);
    vec![
        (
            "ServerName".to_string(),
            quoted(extra_str(input, "server_name").unwrap_or(&input.name)),
        ),
        (
            "ServerPassword".to_string(),
            quoted(extra_str(input, "server_password").unwrap_or("")),
        ),
        (
            "AdminPassword".to_string(),
            quoted(extra_str(input, "admin_password").unwrap_or("hostable-admin")),
        ),
        ("ServerPlayerMaxNum".to_string(), max_players.to_string()),
        ("PublicPort".to_string(), port.to_string()),
        // RCON is how Hostable saves and talks to the game. It is never opened in the firewall.
        ("RCONEnabled".to_string(), "True".to_string()),
        ("RCONPort".to_string(), rcon_port.to_string()),
        ("RESTAPIEnabled".to_string(), "False".to_string()),
    ]
}

fn render_start_script(input: &CreateServerInput) -> String {
    let port = super::extra_port(input, "port", 8211);
    let max_players = input
        .extra
        .get("max_players")
        .and_then(|value| value.as_u64())
        .unwrap_or(8);
    format!(
        "#!/usr/bin/env bash\nset -euo pipefail\ncd \"$(dirname \"$0\")\"\n# steamclient.so is looked up in ~/.steam/sdk64; link it if steamcmd left it elsewhere.\nif [ ! -e \"$HOME/.steam/sdk64/steamclient.so\" ]; then\n  for candidate in \"$HOME/.local/share/Steam/steamcmd/linux64/steamclient.so\" /usr/lib/games/steam/linux64/steamclient.so; do\n    if [ -e \"$candidate\" ]; then mkdir -p \"$HOME/.steam/sdk64\" && ln -sf \"$candidate\" \"$HOME/.steam/sdk64/steamclient.so\"; break; fi\n  done\nfi\nexec ./PalServer.sh -port={port} -players={max_players} -useperfthreads -NoAsyncLoadingThread -UseMultithreadForDS\n"
    )
}

fn render_systemd_unit(input: &CreateServerInput) -> String {
    super::render_unit(
        &format!("Hostable Palworld server {}", input.name),
        "hostable",
        &input.install_path,
        &format!("{}/start.sh", input.install_path.display()),
        180,
    )
}

// ---- PalWorldSettings.ini: OptionSettings=(Key=Value,Key=Value,...) -------------

/// Parse the single `OptionSettings=(...)` line into ordered key/value pairs.
/// Returns `None` when the file has no usable OptionSettings line (for example the
/// empty file the game generates on first boot).
pub fn parse_option_settings(contents: &str) -> Option<Vec<(String, String)>> {
    let line = contents
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("OptionSettings="))?;
    let inner = line
        .strip_prefix("OptionSettings=")?
        .trim()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    let pairs = split_top_level(inner)
        .into_iter()
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    (!pairs.is_empty()).then_some(pairs)
}

/// Set or add options, keeping every other option (and the order) untouched.
/// If `contents` has no OptionSettings line the result contains only `updates`.
pub fn patch_option_settings(contents: &str, updates: &[(String, String)]) -> String {
    let mut pairs = parse_option_settings(contents).unwrap_or_default();
    for (key, value) in updates {
        if let Some(existing) = pairs.iter_mut().find(|(k, _)| k == key) {
            existing.1 = value.clone();
        } else {
            pairs.push((key.clone(), value.clone()));
        }
    }
    let body = pairs
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("[/Script/Pal.PalGameWorldSettings]\nOptionSettings=({body})\n")
}

fn split_top_level(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0u32;
    let mut in_quotes = false;
    for ch in input.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            '(' if !in_quotes => {
                depth += 1;
                current.push(ch);
            }
            ')' if !in_quotes => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if !in_quotes && depth == 0 => {
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

fn safe_config_path(root: &Path, requested: &Path) -> Result<PathBuf> {
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    if !candidate.starts_with(root)
        || candidate
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(HostableError::InvalidInput(format!(
            "config path must stay inside {}",
            root.display()
        )));
    }
    // Only the INI is editable through the API.
    if candidate != root.join(SETTINGS_RELATIVE) {
        return Err(HostableError::InvalidInput(
            "only PalWorldSettings.ini can be edited".to_string(),
        ));
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServerType;

    const DEFAULTS: &str = "[/Script/Pal.PalGameWorldSettings]\nOptionSettings=(Difficulty=None,ServerName=\"Default Palworld Server\",CrossplayPlatforms=(Steam,Xbox,PS5,Mac),BaseCampWorkerMaxNum=15,AdminPassword=\"\")\n";

    fn input() -> CreateServerInput {
        CreateServerInput {
            name: "Friends Pals".to_string(),
            install_path: PathBuf::from("/srv/hostable/servers/friends-pals"),
            server_type: ServerType::Palworld,
            game_version: None,
            loader_version: None,
            build_version: None,
            memory_mb: None,
            accepted_license: true,
            extra: serde_json::json!({ "admin_password": "admin-secret-123" }),
        }
    }

    #[test]
    fn parses_nested_parentheses_and_quotes() {
        let pairs = parse_option_settings(DEFAULTS).unwrap();
        assert_eq!(pairs.len(), 5);
        assert!(pairs.contains(&(
            "CrossplayPlatforms".to_string(),
            "(Steam,Xbox,PS5,Mac)".to_string()
        )));
        assert!(pairs.contains(&(
            "ServerName".to_string(),
            "\"Default Palworld Server\"".to_string()
        )));
    }

    #[test]
    fn empty_first_boot_file_has_no_settings() {
        assert!(parse_option_settings("").is_none());
        assert!(parse_option_settings("[/Script/Pal.PalGameWorldSettings]\n").is_none());
    }

    #[test]
    fn patching_keeps_other_options_and_order() {
        let patched = patch_option_settings(
            DEFAULTS,
            &[
                ("BaseCampWorkerMaxNum".to_string(), "20".to_string()),
                ("RCONEnabled".to_string(), "True".to_string()),
            ],
        );
        assert!(patched.contains("Difficulty=None,ServerName=\"Default Palworld Server\""));
        assert!(patched.contains("CrossplayPlatforms=(Steam,Xbox,PS5,Mac)"));
        assert!(patched.contains("BaseCampWorkerMaxNum=20"));
        assert!(patched.ends_with("RCONEnabled=True)\n"));
        assert_eq!(parse_option_settings(&patched).unwrap().len(), 6);
    }

    #[test]
    fn install_plan_seeds_settings_and_opens_only_the_game_port() {
        let plan = PalworldAdapter.plan_install(input()).unwrap();
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::PatchPalworldSettings { seed_from: Some(seed), .. }
                if seed.ends_with("DefaultPalWorldSettings.ini")
        )));
        let ports = plan
            .actions
            .iter()
            .filter_map(|action| match action {
                HostAction::UfwAllow { port, .. } => Some(*port),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(ports, vec![8211]);
    }

    #[test]
    fn counts_players_from_show_players_reply() {
        assert_eq!(count_palworld_players("name,playeruid,steamid\n"), 0);
        assert_eq!(
            count_palworld_players("name,playeruid,steamid\nAna,123,7656\nBo,456,7657\n"),
            2
        );
    }

    #[test]
    fn reads_the_steam_build_id() {
        let dir = std::env::temp_dir().join(format!("hostable-pal-acf-{}", std::process::id()));
        fs::create_dir_all(dir.join("steamapps")).unwrap();
        fs::write(
            dir.join(format!("steamapps/appmanifest_{PALWORLD_APP_ID}.acf")),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"2394010\"\n\t\"buildid\"\t\t\"24370498\"\n}\n",
        )
        .unwrap();
        assert_eq!(read_build_id(&dir).as_deref(), Some("24370498"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn install_requires_license_acceptance() {
        let mut input = input();
        input.accepted_license = false;
        assert!(PalworldAdapter.plan_install(input).is_err());
    }

    #[test]
    fn diagnose_flags_empty_settings_and_world_option_override() {
        let dir = std::env::temp_dir().join(format!("hostable-pal-{}", std::process::id()));
        let world = dir.join("Pal/Saved/SaveGames/0/ABC");
        fs::create_dir_all(&world).unwrap();
        fs::create_dir_all(dir.join("Pal/Saved/Config/LinuxServer")).unwrap();
        fs::write(dir.join(SETTINGS_RELATIVE), "").unwrap();
        fs::write(world.join("WorldOption.sav"), "x").unwrap();
        let findings = diagnose(&dir);
        assert_eq!(findings.len(), 2, "{findings:?}");
        fs::remove_dir_all(&dir).ok();
    }
}
