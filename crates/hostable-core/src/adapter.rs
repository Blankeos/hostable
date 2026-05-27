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
    fn plan_import(&self, detected: DetectedServer) -> Result<ActionPlan>;

    fn read_config(&self, server: &ServerRecord) -> Result<ConfigDocument>;
    fn write_config(&self, server: &ServerRecord, patch: ConfigPatch) -> Result<()>;

    fn backup_spec(&self, server: &ServerRecord) -> BackupSpec;
    fn health_check(&self, server: &ServerRecord) -> Result<HealthStatus>;

    fn console(&self) -> Option<&dyn ConsoleCapability> {
        None
    }

    fn modrinth(&self) -> Option<&dyn ModrinthCapability> {
        None
    }
}

pub trait ConsoleCapability: Send + Sync {
    fn command_examples(&self) -> Vec<&'static str>;
    fn send_command(&self, server: &ServerRecord, command: &str) -> Result<ConsoleCommandResult>;
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerType {
    Vanilla,
    Paper,
    Fabric,
    ArkSurvivalEvolved,
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
