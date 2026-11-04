use super::slugify;
use crate::actions::{ActionPlan, HostAction};
use crate::adapter::{
    ConfigDocument, ConfigFile, ConfigFormat, ConfigPatch, ConsoleCapability, CreateServerInput,
    DetectedServer, DetectionConfidence, GameAdapter, GameKind, HealthStatus, ModrinthCapability,
    PortSpec, Protocol, Requirements, ServerState, ServerType,
};
use crate::backup::{BackupHook, BackupSpec};
use crate::registry::ServerRecord;
use crate::{HostableError, Result};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Default)]
pub struct MinecraftAdapter;

impl GameAdapter for MinecraftAdapter {
    fn kind(&self) -> GameKind {
        GameKind::Minecraft
    }

    fn display_name(&self) -> &'static str {
        "Minecraft"
    }

    fn default_ports(&self) -> Vec<PortSpec> {
        vec![PortSpec {
            port: 25565,
            protocol: Protocol::Tcp,
            label: "Minecraft server".to_string(),
        }]
    }

    fn requirements(&self) -> Requirements {
        Requirements {
            packages: vec![
                "openjdk-17-jre-headless".to_string(),
                "curl".to_string(),
                "python3".to_string(),
                "unzip".to_string(),
                "ufw".to_string(),
            ],
            architectures: Vec::new(),
            apt_components: Vec::new(),
            min_memory_mb: 2 * 1024,
            recommended_memory_mb: 6 * 1024,
            min_disk_mb: 3 * 1024,
        }
    }

    fn detect_existing(&self, path: &Path) -> Result<DetectedServer> {
        let properties = path.join("server.properties");
        if !properties.exists() {
            return Err(HostableError::InvalidInput(format!(
                "{} does not look like a Minecraft server",
                path.display()
            )));
        }

        let server_type = if crate::minecraft_admin::has_fabric_launcher(path) {
            Some(ServerType::Fabric)
        } else if path.join("paper.jar").exists() {
            Some(ServerType::Paper)
        } else {
            Some(ServerType::Vanilla)
        };

        let properties_text = fs::read_to_string(&properties).unwrap_or_default();
        let property = |key: &str| {
            properties_text.lines().find_map(|line| {
                let (name, value) = line.split_once('=')?;
                (name.trim() == key).then(|| value.trim().to_string())
            })
        };
        let game_port = property("server-port")
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(25565);
        let mut extra = serde_json::Map::new();
        if property("enable-rcon").as_deref() == Some("true")
            && let Some(password) = property("rcon.password").filter(|value| !value.is_empty())
        {
            extra.insert("rcon_password".to_string(), password.into());
            if let Some(port) = property("rcon.port").and_then(|value| value.parse::<u16>().ok()) {
                extra.insert("rcon_port".to_string(), port.into());
            }
        }

        Ok(DetectedServer {
            kind: GameKind::Minecraft,
            install_path: path.to_path_buf(),
            server_type,
            confidence: DetectionConfidence::High,
            ports: vec![PortSpec {
                port: game_port,
                protocol: Protocol::Tcp,
                label: "Minecraft server".to_string(),
            }],
            extra: serde_json::Value::Object(extra),
        })
    }

    fn plan_install(&self, input: CreateServerInput) -> Result<ActionPlan> {
        if !input.accepted_license {
            return Err(HostableError::InvalidInput(
                "Minecraft EULA must be accepted before installation".to_string(),
            ));
        }

        let mut plan = ActionPlan::new(format!("Create Minecraft server '{}'", input.name));
        plan.actions.push(HostAction::AptInstall {
            packages: self.requirements().packages,
        });
        plan.actions.push(HostAction::CreateUser {
            user: "hostable".to_string(),
            system: true,
        });
        plan.actions.push(HostAction::CreateDirectory {
            path: input.install_path.clone(),
            owner: "hostable".to_string(),
        });
        add_server_jar_download(&mut plan, &input)?;
        plan.actions.push(HostAction::WriteFile {
            path: input.install_path.join("server.properties"),
            contents: render_server_properties(&input),
            owner: "hostable".to_string(),
            // holds the RCON password
            mode: 0o600,
        });
        plan.actions.push(HostAction::WriteFile {
            path: input.install_path.join("eula.txt"),
            contents: "eula=true\n".to_string(),
            owner: "hostable".to_string(),
            mode: 0o644,
        });
        plan.actions.push(HostAction::WriteFile {
            path: input.install_path.join("start.sh"),
            contents: render_start_script(&input),
            owner: "hostable".to_string(),
            mode: 0o755,
        });
        add_modrinth_actions(&mut plan, &input);
        plan.actions.push(HostAction::UfwAllow {
            port: super::extra_port(&input, "port", 25565),
            protocol: Protocol::Tcp,
        });
        plan.actions.push(HostAction::RenderSystemdUnit {
            name: format!("hostable-{}.service", slugify(&input.name)),
            contents: render_systemd_unit(&input),
        });
        let unit = format!("hostable-{}.service", slugify(&input.name));
        plan.actions.push(HostAction::SystemdDaemonReload);
        plan.actions.push(HostAction::SystemdEnable { unit });
        Ok(plan)
    }

    fn plan_import(&self, detected: DetectedServer, name: &str) -> Result<ActionPlan> {
        let root = &detected.install_path;
        // Prefer the launch script the owner already uses (it knows their RAM and flags).
        let script = ["start.sh", "run.sh"]
            .iter()
            .map(|file| root.join(file))
            .find(|path| path.exists());
        let exec_start = match script {
            Some(script) => format!("/usr/bin/env bash {}", script.display()),
            None => {
                let jar = ["fabric-server-launch.jar", "paper.jar", "server.jar"]
                    .iter()
                    .find(|file| root.join(file).exists())
                    .ok_or_else(|| {
                        HostableError::InvalidInput(
                            "no start.sh, run.sh or server jar found in this folder".to_string(),
                        )
                    })?;
                format!(
                    "/usr/bin/java -Xms1G -Xmx4G -jar {} nogui",
                    root.join(jar).display()
                )
            }
        };
        super::import_plan(
            "Import Minecraft server",
            name,
            &format!("Hostable Minecraft server {name}"),
            root,
            &exec_start,
            &detected.ports,
            120,
        )
    }

    fn read_config(&self, server: &ServerRecord) -> Result<ConfigDocument> {
        let root = &server.install_path;
        let mut files = Vec::new();
        for (relative, format) in [
            ("server.properties", ConfigFormat::Properties),
            ("eula.txt", ConfigFormat::Properties),
            ("start.sh", ConfigFormat::Text),
            ("ops.json", ConfigFormat::Json),
            ("whitelist.json", ConfigFormat::Json),
            ("banned-players.json", ConfigFormat::Json),
            ("banned-ips.json", ConfigFormat::Json),
            ("hostable.minecraft.mods.json", ConfigFormat::Json),
        ] {
            let path = root.join(relative);
            if path.exists() {
                files.push(ConfigFile {
                    path,
                    format,
                    contents: fs::read_to_string(root.join(relative))?,
                });
            }
        }
        Ok(ConfigDocument {
            title: format!("Minecraft config for {}", server.name),
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
        let root = &server.install_path;
        BackupSpec {
            include_paths: vec![
                root.join("world"),
                root.join("server.properties"),
                root.join("eula.txt"),
                root.join("config"),
                root.join("defaultconfigs"),
                root.join("ops.json"),
                root.join("whitelist.json"),
                root.join("banned-players.json"),
                root.join("banned-ips.json"),
                root.join("hostable.minecraft.mods.json"),
            ],
            exclude_paths: vec![root.join("mods"), root.join(".fabric"), root.join("cache")],
            pre_copy_hook: Some(BackupHook::MinecraftRconSaveAll),
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

    fn modrinth(&self) -> Option<&dyn ModrinthCapability> {
        Some(self)
    }
}

impl ConsoleCapability for MinecraftAdapter {
    fn command_examples(&self) -> Vec<&'static str> {
        vec![
            "say Restarting in 5 minutes",
            "save-all",
            "whitelist add playername",
        ]
    }

    fn send_command(
        &self,
        server: &ServerRecord,
        command: &str,
    ) -> Result<crate::adapter::ConsoleCommandResult> {
        let password = server
            .adapter_config
            .pointer("/extra/rcon_password")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                HostableError::InvalidInput("server has no Minecraft RCON password".to_string())
            })?;
        let port = server
            .adapter_config
            .pointer("/extra/rcon_port")
            .and_then(|value| value.as_u64())
            .unwrap_or(25575);
        let message = crate::rcon::send_command(&format!("127.0.0.1:{port}"), password, command)?;
        Ok(crate::adapter::ConsoleCommandResult {
            accepted: true,
            message,
        })
    }

    fn save_command(&self) -> &'static str {
        "save-all"
    }

    fn online_players(&self, server: &ServerRecord) -> Result<Option<u32>> {
        let reply = self.send_command(server, "list")?.message;
        Ok(Self::parse_player_count(&reply))
    }
}

impl MinecraftAdapter {
    fn parse_player_count(reply: &str) -> Option<u32> {
        // "There are 3 of a max of 20 players online: a, b, c"
        let rest = reply.split("There are ").nth(1)?;
        rest.split_whitespace().next()?.parse().ok()
    }
}

impl ModrinthCapability for MinecraftAdapter {
    fn supports_modpacks(&self) -> bool {
        true
    }

    fn supports_individual_mods(&self) -> bool {
        true
    }
}

fn render_server_properties(input: &CreateServerInput) -> String {
    let port = super::extra_port(input, "port", 25565);
    let rcon_port = super::extra_port(input, "rcon_port", 25575);
    let motd = input
        .extra
        .get("motd")
        .and_then(|value| value.as_str())
        .unwrap_or("A Hostable Minecraft server");
    let online_mode = input
        .extra
        .get("online_mode")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);

    let rcon_password = input
        .extra
        .get("rcon_password")
        .and_then(|value| value.as_str())
        .unwrap_or("hostable-local-rcon");
    let max_players = input
        .extra
        .get("max_players")
        .and_then(|value| value.as_u64())
        .unwrap_or(20);
    let view_distance = input
        .extra
        .get("view_distance")
        .and_then(|value| value.as_u64())
        .unwrap_or(10);
    let simulation_distance = input
        .extra
        .get("simulation_distance")
        .and_then(|value| value.as_u64())
        .unwrap_or(10);

    format!(
        "motd={motd}\nserver-port={port}\nmax-players={max_players}\nview-distance={view_distance}\nsimulation-distance={simulation_distance}\nenable-rcon=true\nrcon.port={rcon_port}\nrcon.password={rcon_password}\nonline-mode={online_mode}\n"
    )
}

fn render_start_script(input: &CreateServerInput) -> String {
    let memory_mb = input.memory_mb.unwrap_or(4096);
    let min_mb = memory_mb.min(1024);
    format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nexec /usr/bin/java -Xms{min_mb}M -Xmx{memory_mb}M -XX:+UseG1GC -XX:+ParallelRefProcEnabled -XX:MaxGCPauseMillis=200 -XX:+UnlockExperimentalVMOptions -XX:+DisableExplicitGC -jar server.jar nogui\n"
    )
}

fn render_systemd_unit(input: &CreateServerInput) -> String {
    super::render_unit(
        &format!("Hostable Minecraft server {}", input.name),
        "hostable",
        &input.install_path,
        &format!("{}/start.sh", input.install_path.display()),
        120,
    )
}

fn add_server_jar_download(plan: &mut ActionPlan, input: &CreateServerInput) -> Result<()> {
    match input.server_type {
        ServerType::Vanilla => {
            plan.actions.push(HostAction::RunCommand {
                program: "python3".to_string(),
                args: vec![
                    "-c".to_string(),
                    VANILLA_DOWNLOAD_SCRIPT.to_string(),
                    input
                        .game_version
                        .clone()
                        .unwrap_or_else(|| "latest".to_string()),
                ],
                user: Some("hostable".to_string()),
                cwd: Some(input.install_path.clone()),
            });
        }
        ServerType::Paper => {
            plan.actions.push(HostAction::RunCommand {
                program: "python3".to_string(),
                args: vec![
                    "-c".to_string(),
                    PAPER_DOWNLOAD_SCRIPT.to_string(),
                    input
                        .game_version
                        .clone()
                        .unwrap_or_else(|| "1.20.4".to_string()),
                    input
                        .build_version
                        .clone()
                        .unwrap_or_else(|| "latest".to_string()),
                ],
                user: Some("hostable".to_string()),
                cwd: Some(input.install_path.clone()),
            });
        }
        ServerType::Fabric => {
            let version = input
                .game_version
                .clone()
                .unwrap_or_else(|| "1.20.4".to_string());
            let loader = input
                .loader_version
                .clone()
                .unwrap_or_else(|| "0.17.2".to_string());
            let installer = input
                .extra
                .get("fabric_installer_version")
                .and_then(|value| value.as_str())
                .unwrap_or("1.0.1");
            plan.actions.push(HostAction::RunCommand {
                program: "curl".to_string(),
                args: vec![
                    "-fL".to_string(),
                    format!("https://meta.fabricmc.net/v2/versions/loader/{version}/{loader}/{installer}/server/jar"),
                    "-o".to_string(),
                    "server.jar".to_string(),
                ],
                user: Some("hostable".to_string()),
                cwd: Some(input.install_path.clone()),
            });
        }
        ServerType::ArkSurvivalEvolved | ServerType::Palworld => {
            return Err(HostableError::InvalidInput(
                "this server type cannot be used with Minecraft".to_string(),
            ));
        }
    }
    Ok(())
}

fn add_modrinth_actions(plan: &mut ActionPlan, input: &CreateServerInput) {
    let Some(modrinth) = input.extra.get("modrinth") else {
        write_modrinth_metadata(plan, input, serde_json::json!({ "source": "none" }));
        return;
    };

    plan.actions.push(HostAction::CreateDirectory {
        path: input.install_path.join("mods"),
        owner: "hostable".to_string(),
    });

    if let Some(url) = modrinth.get("modpack_url").and_then(|value| value.as_str()) {
        plan.actions.push(HostAction::RunCommand {
            program: "curl".to_string(),
            args: vec![
                "-fL".to_string(),
                url.to_string(),
                "-o".to_string(),
                "hostable-modpack.mrpack".to_string(),
            ],
            user: Some("hostable".to_string()),
            cwd: Some(input.install_path.clone()),
        });
        plan.actions.push(HostAction::RunCommand {
            program: "curl".to_string(),
            args: vec![
                "-fL".to_string(),
                "https://github.com/nothub/mrpack-install/releases/download/v0.16.10/mrpack-install-linux".to_string(),
                "-o".to_string(),
                "mrpack-install-linux".to_string(),
            ],
            user: Some("hostable".to_string()),
            cwd: Some(input.install_path.clone()),
        });
        plan.actions.push(HostAction::RunCommand {
            program: "chmod".to_string(),
            args: vec!["+x".to_string(), "mrpack-install-linux".to_string()],
            user: Some("hostable".to_string()),
            cwd: Some(input.install_path.clone()),
        });
        plan.actions.push(HostAction::RunCommand {
            program: "./mrpack-install-linux".to_string(),
            args: vec![
                "hostable-modpack.mrpack".to_string(),
                "--server-dir".to_string(),
                ".".to_string(),
            ],
            user: Some("hostable".to_string()),
            cwd: Some(input.install_path.clone()),
        });
    }

    if let Some(mods) = modrinth.get("mods").and_then(|value| value.as_array()) {
        for item in mods {
            let Some(url) = item.get("download_url").and_then(|value| value.as_str()) else {
                continue;
            };
            let filename = item
                .get("filename")
                .and_then(|value| value.as_str())
                .unwrap_or("mod.jar");
            plan.actions.push(HostAction::RunCommand {
                program: "curl".to_string(),
                args: vec![
                    "-fL".to_string(),
                    url.to_string(),
                    "-o".to_string(),
                    format!("mods/{filename}"),
                ],
                user: Some("hostable".to_string()),
                cwd: Some(input.install_path.clone()),
            });
        }
    }

    write_modrinth_metadata(plan, input, modrinth.clone());
}

fn write_modrinth_metadata(
    plan: &mut ActionPlan,
    input: &CreateServerInput,
    metadata: serde_json::Value,
) {
    plan.actions.push(HostAction::WriteFile {
        path: input.install_path.join("hostable.minecraft.mods.json"),
        contents: serde_json::to_string_pretty(&metadata).expect("modrinth metadata serializes"),
        owner: "hostable".to_string(),
        mode: 0o644,
    });
}

const VANILLA_DOWNLOAD_SCRIPT: &str = r#"
import json
import socket
import sys
import urllib.request

_getaddrinfo = socket.getaddrinfo
def getaddrinfo_ipv4_first(*args, **kwargs):
    results = _getaddrinfo(*args, **kwargs)
    ipv4 = [item for item in results if item[0] == socket.AF_INET]
    return ipv4 or results
socket.getaddrinfo = getaddrinfo_ipv4_first

version = sys.argv[1]
with urllib.request.urlopen("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json") as response:
    manifest = json.load(response)
if version == "latest":
    version = manifest["latest"]["release"]
match = next((item for item in manifest["versions"] if item["id"] == version), None)
if match is None:
    raise SystemExit(f"Minecraft version not found: {version}")
with urllib.request.urlopen(match["url"]) as response:
    details = json.load(response)
urllib.request.urlretrieve(details["downloads"]["server"]["url"], "server.jar")
"#;

const PAPER_DOWNLOAD_SCRIPT: &str = r#"
import json
import socket
import sys
import urllib.request

_getaddrinfo = socket.getaddrinfo
def getaddrinfo_ipv4_first(*args, **kwargs):
    results = _getaddrinfo(*args, **kwargs)
    ipv4 = [item for item in results if item[0] == socket.AF_INET]
    return ipv4 or results
socket.getaddrinfo = getaddrinfo_ipv4_first

version = sys.argv[1]
build = sys.argv[2]
with urllib.request.urlopen(f"https://api.papermc.io/v2/projects/paper/versions/{version}") as response:
    version_info = json.load(response)
if build == "latest":
    build = str(version_info["builds"][-1])
url = f"https://api.papermc.io/v2/projects/paper/versions/{version}/builds/{build}/downloads/paper-{version}-{build}.jar"
urllib.request.urlretrieve(url, "server.jar")
"#;

fn safe_config_path(root: &Path, requested: &Path) -> Result<PathBuf> {
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(HostableError::InvalidInput(
            "config path cannot contain ..".to_string(),
        ));
    }
    let path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    if !path.starts_with(root) {
        return Err(HostableError::InvalidInput(
            "config path must stay inside the server folder".to_string(),
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod player_tests {
    use super::*;

    #[test]
    fn parses_the_list_reply() {
        assert_eq!(
            MinecraftAdapter::parse_player_count("There are 0 of a max of 20 players online: "),
            Some(0)
        );
        assert_eq!(
            MinecraftAdapter::parse_player_count(
                "There are 3 of a max of 20 players online: a, b, c"
            ),
            Some(3)
        );
        assert_eq!(
            MinecraftAdapter::parse_player_count("Unknown command"),
            None
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::HostAction;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn input(server_type: ServerType) -> CreateServerInput {
        CreateServerInput {
            name: "friends".to_string(),
            install_path: "/srv/hostable/servers/friends".into(),
            server_type,
            game_version: Some("1.20.4".to_string()),
            loader_version: None,
            build_version: None,
            memory_mb: Some(4096),
            accepted_license: true,
            extra: serde_json::json!({
                "motd": "friends only",
                "rcon_password": "test-rcon"
            }),
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hostable-minecraft-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn refuses_install_without_eula_acceptance() {
        let adapter = MinecraftAdapter;
        let result = adapter.plan_install(CreateServerInput {
            name: "friends".to_string(),
            install_path: "/srv/hostable/servers/friends".into(),
            server_type: ServerType::Fabric,
            game_version: Some("1.20.4".to_string()),
            loader_version: None,
            build_version: None,
            memory_mb: Some(4096),
            accepted_license: false,
            extra: serde_json::json!({}),
        });

        assert!(result.is_err());
    }

    #[test]
    fn plans_vanilla_paper_and_fabric_server_downloads() {
        let adapter = MinecraftAdapter;

        let vanilla = adapter.plan_install(input(ServerType::Vanilla)).unwrap();
        assert!(vanilla.actions.iter().any(|action| matches!(
            action,
            HostAction::RunCommand { program, args, cwd, .. }
                if program == "python3"
                    && cwd.as_deref() == Some(Path::new("/srv/hostable/servers/friends"))
                    && args.iter().any(|arg| arg.contains("piston-meta.mojang.com"))
                    && args.iter().any(|arg| arg == "1.20.4")
        )));

        let paper = adapter.plan_install(input(ServerType::Paper)).unwrap();
        assert!(paper.actions.iter().any(|action| matches!(
            action,
            HostAction::RunCommand { program, args, .. }
                if program == "python3"
                    && args.iter().any(|arg| arg.contains("api.papermc.io"))
                    && args.iter().any(|arg| arg == "latest")
        )));

        let fabric = adapter.plan_install(input(ServerType::Fabric)).unwrap();
        assert!(fabric.actions.iter().any(|action| matches!(
            action,
            HostAction::RunCommand { program, args, .. }
                if program == "curl"
                    && args.iter().any(|arg| arg.contains("meta.fabricmc.net"))
                    && args.iter().any(|arg| arg == "server.jar")
        )));
        assert!(fabric.actions.iter().any(|action| matches!(
            action,
            HostAction::WriteFile { path, contents, .. }
                if path.ends_with("server.properties")
                    && contents.contains("motd=friends only")
                    && contents.contains("enable-rcon=true")
                    && contents.contains("rcon.password=test-rcon")
        )));
        assert!(fabric.actions.iter().any(|action| matches!(
            action,
            HostAction::WriteFile { path, contents, .. }
                if path.ends_with("eula.txt") && contents == "eula=true\n"
        )));
        assert!(fabric.actions.iter().any(|action| matches!(
            action,
            HostAction::UfwAllow {
                port: 25565,
                protocol: Protocol::Tcp,
            }
        )));
        assert!(fabric.actions.iter().any(|action| matches!(
            action,
            HostAction::SystemdEnable { unit } if unit == "hostable-friends.service"
        )));
    }

    #[test]
    fn plans_modrinth_modpack_and_individual_mod_installs() {
        let adapter = MinecraftAdapter;
        let mut request = input(ServerType::Fabric);
        request.extra = serde_json::json!({
            "modrinth": {
                "modpack_url": "https://cdn.modrinth.test/pack.mrpack",
                "modpack_project_id": "pack-project",
                "mods": [
                    {
                        "download_url": "https://cdn.modrinth.test/mod.jar",
                        "filename": "cool-mod.jar",
                        "project_id": "cool-mod"
                    }
                ]
            }
        });

        let plan = adapter.plan_install(request).unwrap();
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::CreateDirectory { path, .. } if path.ends_with("mods")
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::RunCommand { program, args, .. }
                if program == "curl"
                    && args.iter().any(|arg| arg == "https://cdn.modrinth.test/pack.mrpack")
                    && args.iter().any(|arg| arg == "hostable-modpack.mrpack")
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::RunCommand { program, args, .. }
                if program == "curl"
                    && args.iter().any(|arg| arg.contains("mrpack-install"))
                    && args.iter().any(|arg| arg == "mrpack-install-linux")
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::RunCommand { program, args, .. }
                if program == "./mrpack-install-linux"
                    && args.iter().any(|arg| arg == "hostable-modpack.mrpack")
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::RunCommand { program, args, .. }
                if program == "curl"
                    && args.iter().any(|arg| arg == "https://cdn.modrinth.test/mod.jar")
                    && args.iter().any(|arg| arg == "mods/cool-mod.jar")
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::WriteFile { path, contents, .. }
                if path.ends_with("hostable.minecraft.mods.json")
                    && contents.contains("pack-project")
                    && contents.contains("cool-mod.jar")
        )));
    }

    #[test]
    fn reads_and_writes_config_without_escaping_server_folder() {
        let adapter = MinecraftAdapter;
        let root = temp_root("config");
        fs::write(root.join("server.properties"), "motd=old\n").unwrap();
        fs::write(root.join("eula.txt"), "eula=true\n").unwrap();
        let server = ServerRecord {
            id: crate::ServerId("mc".to_string()),
            name: "mc".to_string(),
            game: GameKind::Minecraft,
            install_path: root.clone(),
            service_name: "hostable-mc.service".to_string(),
            ports: adapter.default_ports(),
            adapter_config: serde_json::json!({}),
        };

        let document = adapter.read_config(&server).unwrap();
        assert_eq!(document.files.len(), 2);
        assert!(
            document
                .files
                .iter()
                .any(|file| file.path.ends_with("server.properties")
                    && file.format == ConfigFormat::Properties)
        );

        adapter
            .write_config(
                &server,
                ConfigPatch {
                    files: vec![ConfigFile {
                        path: PathBuf::from("server.properties"),
                        format: ConfigFormat::Properties,
                        contents: "motd=new\n".to_string(),
                    }],
                },
            )
            .unwrap();
        assert_eq!(
            fs::read_to_string(root.join("server.properties")).unwrap(),
            "motd=new\n"
        );

        let traversal = adapter.write_config(
            &server,
            ConfigPatch {
                files: vec![ConfigFile {
                    path: PathBuf::from("../outside.properties"),
                    format: ConfigFormat::Properties,
                    contents: "bad=true\n".to_string(),
                }],
            },
        );
        assert!(traversal.is_err());

        let outside = adapter.write_config(
            &server,
            ConfigPatch {
                files: vec![ConfigFile {
                    path: std::env::temp_dir().join("hostable-outside.properties"),
                    format: ConfigFormat::Properties,
                    contents: "bad=true\n".to_string(),
                }],
            },
        );
        assert!(outside.is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn backup_spec_excludes_reproducible_mod_jars() {
        let adapter = MinecraftAdapter;
        let server = ServerRecord {
            id: crate::ServerId("mc".to_string()),
            name: "mc".to_string(),
            game: GameKind::Minecraft,
            install_path: "/srv/hostable/servers/mc".into(),
            service_name: "hostable-mc.service".to_string(),
            ports: adapter.default_ports(),
            adapter_config: serde_json::json!({}),
        };

        let spec = adapter.backup_spec(&server);
        assert!(
            spec.include_paths
                .iter()
                .any(|path| path.ends_with("world"))
        );
        assert!(spec.exclude_paths.iter().any(|path| path.ends_with("mods")));
    }
}
