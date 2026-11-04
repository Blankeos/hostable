pub mod ark_se;
pub mod minecraft;
pub mod palworld;

pub use ark_se::ArkSurvivalEvolvedAdapter;
pub use minecraft::MinecraftAdapter;
pub use palworld::PalworldAdapter;

use crate::adapter::{GameAdapter, GameKind};

/// Every supported game, in the order shown in the UI.
pub const ALL_GAMES: [GameKind; 3] = [
    GameKind::Minecraft,
    GameKind::ArkSurvivalEvolved,
    GameKind::Palworld,
];

/// The one place that maps a game kind to its adapter.
pub fn adapter_for(kind: GameKind) -> &'static dyn GameAdapter {
    static MINECRAFT: MinecraftAdapter = MinecraftAdapter;
    static ARK: ArkSurvivalEvolvedAdapter = ArkSurvivalEvolvedAdapter;
    static PALWORLD: PalworldAdapter = PalworldAdapter;
    match kind {
        GameKind::Minecraft => &MINECRAFT,
        GameKind::ArkSurvivalEvolved => &ARK,
        GameKind::Palworld => &PALWORLD,
    }
}

/// Debian and Ubuntu install steamcmd under /usr/games, which is not on the PATH of
/// systemd services or `runuser`, so always call it by its full path.
pub(crate) const STEAMCMD: &str = "/usr/games/steamcmd";

use crate::actions::{ActionPlan, HostAction};
use crate::{HostableError, Result};
use std::path::Path;

/// systemd unit text shared by every game. `user` owns the process.
pub(crate) fn render_unit(
    description: &str,
    user: &str,
    working_dir: &Path,
    exec_start: &str,
    stop_timeout_secs: u32,
) -> String {
    format!(
        "[Unit]\nDescription={description}\nAfter=network.target\n\n[Service]\nUser={user}\nWorkingDirectory={dir}\nExecStart={exec_start}\nRestart=on-failure\nRestartSec=10\n# SIGTERM is how systemd asks the game to save and quit, so exit 143 is a clean stop.\nSuccessExitStatus=143\nTimeoutStopSec={stop_timeout_secs}\n# Keep a runaway game from taking sshd and the daemon down with it.\nOOMScoreAdjust=300\n\n[Install]\nWantedBy=multi-user.target\n",
        dir = working_dir.display()
    )
}

/// Name of the user that owns `path`, so an imported server keeps running as the
/// same user instead of having its files re-owned.
pub(crate) fn owner_of(path: &Path) -> Result<String> {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata(path)?.uid();
    if uid == 0 {
        return Err(HostableError::InvalidInput(format!(
            "{} is owned by root. Give it to a normal user first (for example `chown -R <user> {}`)",
            path.display(),
            path.display()
        )));
    }
    let output = std::process::Command::new("id")
        .arg("-nu")
        .arg(uid.to_string())
        .output()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || name.is_empty() {
        return Err(HostableError::InvalidInput(format!(
            "could not find the user with uid {uid} that owns {}",
            path.display()
        )));
    }
    Ok(name)
}

/// A port chosen for this server (`extra.port` / `extra.rcon_port`), or the game's default.
pub(crate) fn extra_port(
    input: &crate::adapter::CreateServerInput,
    key: &str,
    default: u16,
) -> u16 {
    input
        .extra
        .get(key)
        .and_then(|value| value.as_u64())
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value >= 1024)
        .unwrap_or(default)
}

/// Owner of an existing folder, or the managed `hostable` user for fresh installs.
pub(crate) fn owner_or_hostable(path: &Path) -> String {
    owner_of(path).unwrap_or_else(|_| "hostable".to_string())
}

pub(crate) fn slugify(input: &str) -> String {
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

/// Plan that adopts an existing server: write a unit that runs it as its current
/// owner and open its ports. It deliberately does NOT enable or start the unit, so
/// it cannot fight an existing service for the same port at boot.
pub(crate) fn import_plan(
    title: &str,
    name: &str,
    description: &str,
    install_path: &Path,
    exec_start: &str,
    ports: &[crate::adapter::PortSpec],
    stop_timeout_secs: u32,
) -> Result<ActionPlan> {
    let user = owner_of(install_path)?;
    let mut plan = ActionPlan::new(title);
    for port in ports {
        plan.actions.push(HostAction::UfwAllow {
            port: port.port,
            protocol: port.protocol,
        });
    }
    plan.actions.push(HostAction::RenderSystemdUnit {
        name: format!("hostable-{}.service", slugify(name)),
        contents: render_unit(
            description,
            &user,
            install_path,
            exec_start,
            stop_timeout_secs,
        ),
    });
    plan.actions.push(HostAction::SystemdDaemonReload);
    Ok(plan)
}
