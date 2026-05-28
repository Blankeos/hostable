use crate::actions::{ActionPlan, HostAction};
use crate::adapter::{
    ConfigDocument, ConfigFile, ConfigFormat, ConfigPatch, ConsoleCapability, CreateServerInput,
    DetectedServer, DetectionConfidence, GameAdapter, GameKind, HealthStatus, PortSpec, Protocol,
    Requirements, ServerState,
};
use crate::backup::BackupSpec;
use crate::registry::ServerRecord;
use crate::{HostableError, Result};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Default)]
pub struct ArkSurvivalEvolvedAdapter;

impl GameAdapter for ArkSurvivalEvolvedAdapter {
    fn kind(&self) -> GameKind {
        GameKind::ArkSurvivalEvolved
    }

    fn display_name(&self) -> &'static str {
        "ARK: Survival Evolved"
    }

    fn default_ports(&self) -> Vec<PortSpec> {
        vec![
            PortSpec {
                port: 7777,
                protocol: Protocol::Udp,
                label: "Game".to_string(),
            },
            PortSpec {
                port: 7778,
                protocol: Protocol::Udp,
                label: "Peer".to_string(),
            },
            PortSpec {
                port: 27015,
                protocol: Protocol::Udp,
                label: "Query".to_string(),
            },
        ]
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
        }
    }

    fn detect_existing(&self, path: &Path) -> Result<DetectedServer> {
        let binary = path.join("ShooterGame/Binaries/Linux/ShooterGameServer");
        if !binary.exists() {
            return Err(HostableError::InvalidInput(format!(
                "{} does not look like an ARK server",
                path.display()
            )));
        }

        Ok(DetectedServer {
            kind: GameKind::ArkSurvivalEvolved,
            install_path: path.to_path_buf(),
            server_type: Some(crate::ServerType::ArkSurvivalEvolved),
            confidence: DetectionConfidence::High,
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
        let mut plan = ActionPlan::new(format!("Create ARK server '{}'", input.name));
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
            path: input.install_path.clone(),
            owner: "hostable".to_string(),
        });
        plan.actions.push(HostAction::RunCommand {
            program: "steamcmd".to_string(),
            args: vec![
                "+force_install_dir".to_string(),
                input.install_path.to_string_lossy().into_owned(),
                "+login".to_string(),
                "anonymous".to_string(),
                "+app_update".to_string(),
                "376030".to_string(),
                "validate".to_string(),
                "+quit".to_string(),
            ],
            user: Some("hostable".to_string()),
            cwd: Some(input.install_path.clone()),
        });
        plan.actions.push(HostAction::CreateDirectory {
            path: input
                .install_path
                .join("ShooterGame/Saved/Config/LinuxServer"),
            owner: "hostable".to_string(),
        });
        plan.actions.push(HostAction::WriteFile {
            path: input
                .install_path
                .join("ShooterGame/Saved/Config/LinuxServer/GameUserSettings.ini"),
            contents: render_game_user_settings(&input),
            owner: "hostable".to_string(),
            mode: 0o644,
        });
        plan.actions.push(HostAction::WriteFile {
            path: input
                .install_path
                .join("ShooterGame/Saved/Config/LinuxServer/Game.ini"),
            contents: render_game_ini(),
            owner: "hostable".to_string(),
            mode: 0o644,
        });
        plan.actions.push(HostAction::WriteFile {
            path: input.install_path.join("start.sh"),
            contents: render_start_script(&input),
            owner: "hostable".to_string(),
            mode: 0o755,
        });
        for port in self.default_ports() {
            plan.actions.push(HostAction::UfwAllow {
                port: port.port,
                protocol: port.protocol,
            });
        }
        let unit = format!("hostable-{}.service", slugify(&input.name));
        plan.actions.push(HostAction::RenderSystemdUnit {
            name: unit.clone(),
            contents: render_systemd_unit(&input),
        });
        plan.actions.push(HostAction::SystemdDaemonReload);
        plan.actions.push(HostAction::SystemdEnable { unit });
        Ok(plan)
    }

    fn plan_import(&self, detected: DetectedServer) -> Result<ActionPlan> {
        Ok(
            ActionPlan::new("Import ARK server").push(HostAction::CreateDirectory {
                path: detected.install_path,
                owner: "hostable".to_string(),
            }),
        )
    }

    fn read_config(&self, server: &ServerRecord) -> Result<ConfigDocument> {
        let root = &server.install_path;
        let mut files = Vec::new();
        for relative in [
            "ShooterGame/Saved/Config/LinuxServer/GameUserSettings.ini",
            "ShooterGame/Saved/Config/LinuxServer/Game.ini",
        ] {
            let path = root.join(relative);
            if path.exists() {
                files.push(ConfigFile {
                    path: path.clone(),
                    format: ConfigFormat::Ini,
                    contents: fs::read_to_string(path)?,
                });
            }
        }
        Ok(ConfigDocument {
            title: format!("ARK config for {}", server.name),
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
            include_paths: vec![server.install_path.join("ShooterGame/Saved")],
            exclude_paths: Vec::new(),
            pre_copy_hook: None,
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
}

impl ConsoleCapability for ArkSurvivalEvolvedAdapter {
    fn command_examples(&self) -> Vec<&'static str> {
        vec!["Broadcast Restarting in 5 minutes", "SaveWorld"]
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
                HostableError::InvalidInput("server has no ARK admin password".to_string())
            })?;
        let port = server
            .adapter_config
            .pointer("/extra/rcon_port")
            .and_then(|value| value.as_u64())
            .unwrap_or(27020);
        let message = crate::rcon::send_command(&format!("127.0.0.1:{port}"), password, command)?;
        Ok(crate::adapter::ConsoleCommandResult {
            accepted: true,
            message,
        })
    }
}

fn render_start_script(input: &CreateServerInput) -> String {
    let session_name = input
        .extra
        .get("session_name")
        .and_then(|value| value.as_str())
        .unwrap_or(&input.name);
    let map = input
        .extra
        .get("map")
        .and_then(|value| value.as_str())
        .unwrap_or("Fjordur");
    let max_players = input
        .extra
        .get("max_players")
        .and_then(|value| value.as_u64())
        .unwrap_or(10);

    format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nexec ./ShooterGame/Binaries/Linux/ShooterGameServer \"{map}?listen?SessionName={session_name}?MaxPlayers={max_players}\" -server -log\n"
    )
}

fn render_game_user_settings(input: &CreateServerInput) -> String {
    let session_name = input
        .extra
        .get("session_name")
        .and_then(|value| value.as_str())
        .unwrap_or(&input.name);
    let admin_password = input
        .extra
        .get("admin_password")
        .and_then(|value| value.as_str())
        .unwrap_or("hostable-admin");
    let server_password = input
        .extra
        .get("server_password")
        .and_then(|value| value.as_str())
        .unwrap_or("");

    format!(
        "[ServerSettings]\nSessionName={session_name}\nServerAdminPassword={admin_password}\nServerPassword={server_password}\nRCONEnabled=True\nRCONPort=27020\n\n[/Script/ShooterGame.ShooterGameUserSettings]\nMasterAudioVolume=1.000000\n"
    )
}

fn render_game_ini() -> String {
    "[/Script/ShooterGame.ShooterGameMode]\n".to_string()
}

fn render_systemd_unit(input: &CreateServerInput) -> String {
    format!(
        r#"[Unit]
Description=Hostable ARK server {name}
After=network.target

[Service]
User=hostable
WorkingDirectory={path}
ExecStart={path}/start.sh
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
"#,
        name = input.name,
        path = input.install_path.display()
    )
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
mod tests {
    use super::*;
    use crate::actions::HostAction;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn input() -> CreateServerInput {
        CreateServerInput {
            name: "island".to_string(),
            install_path: "/srv/hostable/servers/island".into(),
            server_type: crate::ServerType::ArkSurvivalEvolved,
            game_version: None,
            loader_version: None,
            build_version: None,
            memory_mb: None,
            accepted_license: true,
            extra: serde_json::json!({
                "session_name": "Friends Island",
                "admin_password": "admin-secret",
                "server_password": "join-secret",
                "map": "TheIsland",
                "max_players": 12
            }),
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hostable-ark-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn refuses_install_without_steam_license_acceptance() {
        let adapter = ArkSurvivalEvolvedAdapter;
        let mut request = input();
        request.accepted_license = false;

        assert!(adapter.plan_install(request).is_err());
    }

    #[test]
    fn plans_steamcmd_install_config_ports_and_service() {
        let adapter = ArkSurvivalEvolvedAdapter;
        let plan = adapter.plan_install(input()).unwrap();

        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::EnableAptComponent { components }
                if components.contains(&"contrib".to_string())
                    && components.contains(&"non-free".to_string())
                    && components.contains(&"non-free-firmware".to_string())
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::AddArchitecture { arch } if arch == "i386"
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::AptInstall { packages }
                if packages.contains(&"steamcmd".to_string())
                    && packages.contains(&"lib32gcc-s1".to_string())
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::RunCommand { program, args, .. }
                if program == "steamcmd"
                    && args.iter().any(|arg| arg == "+app_update")
                    && args.iter().any(|arg| arg == "376030")
                    && args.iter().any(|arg| arg == "validate")
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::WriteFile { path, contents, .. }
                if path.ends_with("GameUserSettings.ini")
                    && contents.contains("SessionName=Friends Island")
                    && contents.contains("ServerAdminPassword=admin-secret")
                    && contents.contains("RCONEnabled=True")
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::WriteFile { path, contents, .. }
                if path.ends_with("Game.ini")
                    && contents.contains("ShooterGameMode")
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::WriteFile { path, contents, .. }
                if path.ends_with("start.sh")
                    && contents.contains("TheIsland?listen")
                    && contents.contains("MaxPlayers=12")
        )));
        for port in [7777, 7778, 27015] {
            assert!(plan.actions.iter().any(|action| matches!(
                action,
                HostAction::UfwAllow {
                    port: action_port,
                    protocol: Protocol::Udp,
                } if *action_port == port
            )));
        }
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            HostAction::SystemdEnable { unit } if unit == "hostable-island.service"
        )));
    }

    #[test]
    fn reads_and_writes_ini_config_without_escaping_server_folder() {
        let adapter = ArkSurvivalEvolvedAdapter;
        let root = temp_root("config");
        let config_dir = root.join("ShooterGame/Saved/Config/LinuxServer");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("GameUserSettings.ini"),
            "[ServerSettings]\n",
        )
        .unwrap();
        fs::write(
            config_dir.join("Game.ini"),
            "[/Script/ShooterGame.ShooterGameMode]\n",
        )
        .unwrap();
        let server = ServerRecord {
            id: crate::ServerId("ark".to_string()),
            name: "ark".to_string(),
            game: GameKind::ArkSurvivalEvolved,
            install_path: root.clone(),
            service_name: "hostable-ark.service".to_string(),
            ports: adapter.default_ports(),
            adapter_config: serde_json::json!({}),
        };

        let document = adapter.read_config(&server).unwrap();
        assert_eq!(document.files.len(), 2);
        assert!(
            document
                .files
                .iter()
                .all(|file| file.format == ConfigFormat::Ini)
        );

        adapter
            .write_config(
                &server,
                ConfigPatch {
                    files: vec![ConfigFile {
                        path: PathBuf::from("ShooterGame/Saved/Config/LinuxServer/Game.ini"),
                        format: ConfigFormat::Ini,
                        contents: "[/Script/ShooterGame.ShooterGameMode]\nNewSetting=True\n"
                            .to_string(),
                    }],
                },
            )
            .unwrap();
        assert!(
            fs::read_to_string(config_dir.join("Game.ini"))
                .unwrap()
                .contains("NewSetting=True")
        );

        let traversal = adapter.write_config(
            &server,
            ConfigPatch {
                files: vec![ConfigFile {
                    path: PathBuf::from("../Game.ini"),
                    format: ConfigFormat::Ini,
                    contents: "bad=true\n".to_string(),
                }],
            },
        );
        assert!(traversal.is_err());

        let outside = adapter.write_config(
            &server,
            ConfigPatch {
                files: vec![ConfigFile {
                    path: std::env::temp_dir().join("hostable-ark-outside.ini"),
                    format: ConfigFormat::Ini,
                    contents: "bad=true\n".to_string(),
                }],
            },
        );
        assert!(outside.is_err());

        let _ = fs::remove_dir_all(root);
    }
}
