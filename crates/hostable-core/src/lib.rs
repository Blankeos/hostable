pub mod actions;
pub mod adapter;
pub mod backup;
pub mod config;
pub mod error;
pub mod games;
pub mod minecraft_admin;
pub mod modrinth;
pub(crate) mod rcon;
pub mod registry;
pub mod service;

pub use actions::{ActionPlan, HostAction};
pub use adapter::{
    ConfigDocument, ConfigPatch, ConsoleCapability, CreateServerInput, DetectedServer, GameAdapter,
    GameKind, HealthStatus, ModrinthCapability, PortSpec, Protocol, Requirements, ServerType,
};
pub use backup::{BackupHook, BackupManifest, BackupPolicy, BackupSpec, RetentionPolicy, S3Config};
pub use error::{HostableError, Result};
pub use games::{
    ALL_GAMES, ArkSurvivalEvolvedAdapter, MinecraftAdapter, PalworldAdapter, adapter_for, palworld,
};
pub use modrinth::{
    ModrinthFile, ModrinthInstallFile, ModrinthInstallSelection, ModrinthProjectType,
    ModrinthSearchHit, ModrinthSearchResponse, ModrinthVersion,
};
pub use registry::{RegistryStore, ServerId, ServerRecord};
