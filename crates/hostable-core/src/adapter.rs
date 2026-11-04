use crate::Result;
use crate::actions::ActionPlan;
use crate::backup::BackupSpec;
use crate::registry::ServerRecord;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub trait GameAdapter: Send + Sync {
    fn kind(&self) -> GameKind;
    fn display_name(&self) -> &'static str;
    fn default_ports(&self) -> Vec<PortSpec>;
    fn requirements(&self) -> Requirements;

    fn detect_existing(&self, path: &Path) -> Result<DetectedServer>;
    fn plan_install(&self, input: CreateServerInput) -> Result<ActionPlan>;
    fn plan_import(&self, detected: DetectedServer, name: &str) -> Result<ActionPlan>;

    fn read_config(&self, server: &ServerRecord) -> Result<ConfigDocument>;
    fn write_config(&self, server: &ServerRecord, patch: ConfigPatch) -> Result<()>;

    fn backup_spec(&self, server: &ServerRecord) -> BackupSpec;
    fn health_check(&self, server: &ServerRecord) -> Result<HealthStatus>;

    fn console(&self) -> Option<&dyn ConsoleCapability> {
        None
    }

    /// Steps that update the game files in place (the caller stops the service and
    /// takes a backup first). `None` means the game has no in-place updater.
    fn plan_update(&self, _server: &ServerRecord) -> Option<Vec<crate::actions::HostAction>> {
        None
    }

    /// Steam build id currently installed, when the game is installed through Steam.
    fn installed_build(&self, _server: &ServerRecord) -> Option<String> {
        None
    }

    fn modrinth(&self) -> Option<&dyn ModrinthCapability> {
        None
    }
}

pub trait ConsoleCapability: Send + Sync {
    fn command_examples(&self) -> Vec<&'static str>;
    fn send_command(&self, server: &ServerRecord, command: &str) -> Result<ConsoleCommandResult>;

    /// Console command that flushes the world to disk, sent before stops and restarts.
    fn save_command(&self) -> &'static str;

    /// Number of players online right now, or `None` when the game cannot say.
    fn online_players(&self, server: &ServerRecord) -> Result<Option<u32>>;
}

pub trait ModrinthCapability: Send + Sync {
    fn supports_modpacks(&self) -> bool;
    fn supports_individual_mods(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameKind {
    Minecraft,
    ArkSurvivalEvolved,
    Palworld,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerType {
    Vanilla,
    Paper,
    Fabric,
    ArkSurvivalEvolved,
    Palworld,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateServerInput {
    pub name: String,
    pub install_path: PathBuf,
    pub server_type: ServerType,
    pub game_version: Option<String>,
    pub loader_version: Option<String>,
    pub build_version: Option<String>,
    pub memory_mb: Option<u32>,
    pub accepted_license: bool,
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedServer {
    pub kind: GameKind,
    pub install_path: PathBuf,
    pub server_type: Option<ServerType>,
    pub confidence: DetectionConfidence,
    /// Ports the existing server really uses (read from its config).
    #[serde(default)]
    pub ports: Vec<PortSpec>,
    /// Settings found in the existing server (RCON port/password, ...) that
    /// Hostable needs to manage it. Merged into the record's `extra`.
    #[serde(default)]
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirements {
    pub packages: Vec<String>,
    pub architectures: Vec<String>,
    pub apt_components: Vec<String>,
    /// Below this the game is likely to be OOM-killed even with swap.
    #[serde(default)]
    pub min_memory_mb: u32,
    /// What the game vendor recommends for a comfortable server.
    #[serde(default)]
    pub recommended_memory_mb: u32,
    /// Free disk needed to install the game and a few backups.
    #[serde(default)]
    pub min_disk_mb: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortSpec {
    pub port: u16,
    pub protocol: Protocol,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigDocument {
    pub title: String,
    pub files: Vec<ConfigFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigFile {
    pub path: PathBuf,
    pub format: ConfigFormat,
    pub contents: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFormat {
    Properties,
    Ini,
    Json,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigPatch {
    pub files: Vec<ConfigFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthStatus {
    pub state: ServerState,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    Unknown,
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleCommandResult {
    pub accepted: bool,
    pub message: String,
}
