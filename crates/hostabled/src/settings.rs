use anyhow::{Context, Result};
use hostable_core::S3Config;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostableSettings {
    pub s3: Option<S3Config>,
    #[serde(default)]
    pub backup: BackupSettings,
    #[serde(default)]
    pub admin_web: AdminWebSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSettings {
    #[serde(default = "default_keep_last")]
    pub local_keep_last: u32,
    #[serde(default = "default_keep_last")]
    pub s3_keep_last: u32,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            local_keep_last: default_keep_last(),
            s3_keep_last: default_keep_last(),
        }
    }
}

fn default_keep_last() -> u32 {
    5
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdminWebSettings {
    pub enabled: bool,
    pub domain: Option<String>,
}

impl SettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<HostableSettings> {
        if !self.path.exists() {
            return Ok(HostableSettings::default());
        }
        let bytes = fs::read(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse {}", self.path.display()))
    }

    pub fn save(&self, settings: &HostableSettings) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(settings)?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write {}", self.path.display()))
    }
}
