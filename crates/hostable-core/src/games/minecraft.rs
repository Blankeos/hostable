use crate::actions::{ActionPlan, HostAction};
use crate::adapter::{
    ConfigDocument, ConfigPatch, ConsoleCapability, CreateServerInput, DetectedServer,
    DetectionConfidence, GameAdapter, GameKind, HealthStatus, ModrinthCapability, PortSpec,
    Protocol, Requirements, ServerState, ServerType,
};
use crate::backup::{BackupHook, BackupSpec};
use crate::registry::ServerRecord;
use crate::{HostableError, Result};
use std::path::Path;

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
                "openjdk-21-jre-headless".to_string(),
                "curl".to_string(),
                "unzip".to_string(),
            ],
            architectures: Vec::new(),
            apt_components: Vec::new(),
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

        let server_type = if path.join("fabric-server-launch.jar").exists() {
            Some(ServerType::Fabric)
        } else if path.join("paper.jar").exists() {
            Some(ServerType::Paper)
        } else {
            Some(ServerType::Vanilla)
        };

        Ok(DetectedServer {
            kind: GameKind::Minecraft,
            install_path: path.to_path_buf(),
            server_type,
            confidence: DetectionConfidence::High,
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
        plan.actions.push(HostAction::CreateDirectory {
            path: input.install_path.clone(),
            owner: "hostable".to_string(),
        });
        plan.actions.push(HostAction::WriteFile {
            path: input.install_path.join("server.properties"),
            contents: render_server_properties(&input),
            owner: "hostable".to_string(),
            mode: 0o644,
        });
        plan.actions.push(HostAction::WriteFile {
            path: input.install_path.join("eula.txt"),
            contents: "eula=true\n".to_string(),
            owner: "hostable".to_string(),
            mode: 0o644,
        });
        plan.actions.push(HostAction::UfwAllow {
            port: 25565,
            protocol: Protocol::Tcp,
        });
        plan.actions.push(HostAction::RenderSystemdUnit {
            name: format!("hostable-{}.service", slugify(&input.name)),
            contents: render_systemd_unit(&input),
        });
        Ok(plan)
    }

    fn plan_import(&self, detected: DetectedServer) -> Result<ActionPlan> {
        Ok(
            ActionPlan::new("Import Minecraft server").push(HostAction::CreateDirectory {
                path: detected.install_path,
                owner: "hostable".to_string(),
            }),
        )
    }

    fn read_config(&self, _server: &ServerRecord) -> Result<ConfigDocument> {
        Err(HostableError::Unsupported(
            "Minecraft config reading is not implemented yet".to_string(),
        ))
    }

    fn write_config(&self, _server: &ServerRecord, _patch: ConfigPatch) -> Result<()> {
        Err(HostableError::Unsupported(
            "Minecraft config writing is not implemented yet".to_string(),
        ))
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
            detail: "systemd status integration is not implemented yet".to_string(),
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
        _server: &ServerRecord,
        _command: &str,
    ) -> Result<crate::adapter::ConsoleCommandResult> {
        Err(HostableError::Unsupported(
            "Minecraft RCON command execution is not implemented yet".to_string(),
        ))
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

    format!(
        "motd={motd}\nserver-port=25565\nenable-rcon=true\nrcon.port=25575\nonline-mode={online_mode}\n"
    )
}

fn render_systemd_unit(input: &CreateServerInput) -> String {
    let memory_mb = input.memory_mb.unwrap_or(4096);
    format!(
        r#"[Unit]
Description=Hostable Minecraft server {name}
After=network.target

[Service]
User=hostable
WorkingDirectory={path}
ExecStart=/usr/bin/java -Xms{memory_mb}M -Xmx{memory_mb}M -jar server.jar nogui
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_install_without_eula_acceptance() {
        let adapter = MinecraftAdapter;
        let result = adapter.plan_install(CreateServerInput {
            name: "friends".to_string(),
            install_path: "/srv/hostable/servers/friends".into(),
            server_type: ServerType::Fabric,
            game_version: Some("1.21.1".to_string()),
            loader_version: None,
            build_version: None,
            memory_mb: Some(4096),
            accepted_license: false,
            extra: serde_json::json!({}),
        });

        assert!(result.is_err());
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
