use crate::actions::{ActionPlan, HostAction};
use crate::adapter::{
    ConfigDocument, ConfigPatch, ConsoleCapability, CreateServerInput, DetectedServer,
    DetectionConfidence, GameAdapter, GameKind, HealthStatus, PortSpec, Protocol, Requirements,
    ServerState,
};
use crate::backup::BackupSpec;
use crate::registry::ServerRecord;
use crate::{HostableError, Result};
use std::path::Path;

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
            packages: vec!["steamcmd".to_string(), "lib32gcc-s1".to_string()],
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
        plan.actions.push(HostAction::CreateDirectory {
            path: input.install_path.clone(),
            owner: "hostable".to_string(),
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

    fn read_config(&self, _server: &ServerRecord) -> Result<ConfigDocument> {
        Err(HostableError::Unsupported(
            "ARK config reading is not implemented yet".to_string(),
        ))
    }

    fn write_config(&self, _server: &ServerRecord, _patch: ConfigPatch) -> Result<()> {
        Err(HostableError::Unsupported(
            "ARK config writing is not implemented yet".to_string(),
        ))
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
            detail: "systemd status integration is not implemented yet".to_string(),
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
        _server: &ServerRecord,
        _command: &str,
    ) -> Result<crate::adapter::ConsoleCommandResult> {
        Err(HostableError::Unsupported(
            "ARK RCON command execution is not implemented yet".to_string(),
        ))
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
        "#!/usr/bin/env bash\nset -euo pipefail\n./ShooterGame/Binaries/Linux/ShooterGameServer \"{map}?listen?SessionName={session_name}?MaxPlayers={max_players}\" -server -log\n"
    )
}
